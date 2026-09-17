"""Stop gate: decides, when an agent stops, whether its current task is done.

Interface (the Stop hook depends only on this):

    gate = load_gate(name)                   # name from hooks_state/config.json "stop_gate"
    verdict = gate.evaluate(GateInput(...))  # -> Verdict
    verdict.decision in {"done", "continue", "escalate"}

  done      the Stop hook marks the task done (note = verdict.reason) and
            feeds the next task.
  continue  the Stop hook blocks the stop with verdict.reason; after
            config gate_max_continues continues on one task it escalates.
  escalate  the task is marked blocked with verdict.reason and the agent
            stops for real: a lead or human is needed.

Only a deterministic placeholder exists. Another gate must implement the
same `evaluate` and register in GATES.

Test scope (config "gate_tests"):

  task   (default) the task's own acceptance module from the scenario
         (tasks.json `hidden_test`, found through <run>/meta.json and
         task_ids.json), run against a scratch copy of the repository. Other
         tasks' unfinished work can make the full suite fail while this
         task is fine; that must not hold the worker on it or escalate it.
         The full suite still runs, as evidence only (`suite_ok`). A task
         the scenario does not know falls back to the suite.
  suite  `test_command` in the repository, the whole suite (the first
         prototype's behavior).
"""

import json
import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, List, Optional


@dataclass
class GateInput:
    """What the gate sees at a stop."""

    task: dict                   # Tirith task row: id, title, description, paths
    last_message: str            # last_assistant_message from the Stop input
    repo: Path                   # repository root to test (<run>/repo)
    agent: str
    config: dict
    continues: int = 0           # continue verdicts already given for this task
    run: Optional[Path] = None   # run directory; default: repo's parent


@dataclass
class Verdict:
    decision: str                # done | continue | escalate
    reason: str
    evidence: dict = field(default_factory=dict)


STOPWORDS = {"about", "after", "again", "every", "their", "there", "these", "those", "which", "while", "would",
             "should", "could", "still", "other", "added", "passes", "python3", "unittest", "tests", "order",
             "today", "change", "changes"}


def acceptance_criteria(description: str) -> List[str]:
    """Bullet lines after an 'Acceptance criteria:' heading (continuation lines folded in)."""
    lines = (description or "").splitlines()
    start = next((i for i, line in enumerate(lines) if line.strip().lower().startswith("acceptance criteria")), None)
    if start is None:
        return []
    criteria: List[str] = []
    for line in lines[start + 1:]:
        stripped = line.strip()
        if stripped.startswith(("- ", "* ")):
            criteria.append(stripped[2:].strip())
        elif stripped and criteria and line[:1].isspace():
            criteria[-1] += " " + stripped
        elif stripped and criteria:
            break
    return criteria


def criterion_terms(criterion: str) -> List[str]:
    """Identifiers in backticks (their last dotted part, call parentheses dropped)."""
    terms = []
    for span in re.findall(r"`([^`]+)`", criterion):
        head = span.split("(")[0].strip()
        ident = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", head)
        if ident:
            terms.append(ident[-1])
    return terms


def mentioned(criterion: str, message: str) -> bool:
    terms = [t for t in criterion_terms(criterion) if len(t) >= 3]
    if terms:
        return any(re.search(r"(?<![A-Za-z0-9_])%s(?![A-Za-z0-9_])" % re.escape(t), message) for t in terms)
    words = {w for w in re.findall(r"[a-z][a-z_]{4,}", criterion.lower()) if w not in STOPWORDS}
    if not words:
        return True
    lowered = message.lower()
    return sum(1 for w in words if w in lowered) * 2 >= len(words)


def run_tests(repo: Path, config: dict) -> dict:
    try:
        proc = subprocess.run(list(config["test_command"]), cwd=str(repo), capture_output=True, text=True,
                              timeout=config["test_timeout_secs"])
        output = (proc.stderr or "") + (proc.stdout or "")
        return {"ok": proc.returncode == 0, "returncode": proc.returncode, "tail": output[-2500:]}
    except subprocess.TimeoutExpired:
        return {"ok": False, "returncode": None, "tail": "tests timed out after %ss" % config["test_timeout_secs"]}


# Runs one test module and prints {"run", "passed", "failed", "import_error"} (same shape as lib/score.py).
RUNNER = r"""
import json, sys, unittest
out = {"run": 0, "passed": [], "failed": [], "import_error": None}
try:
    suite = unittest.defaultTestLoader.loadTestsFromName(sys.argv[1])
except Exception as err:
    out["import_error"] = "%s: %s" % (type(err).__name__, err)
    print(json.dumps(out)); sys.exit(0)
class Result(unittest.TestResult):
    def addSuccess(self, test):
        super().addSuccess(test); out["passed"].append(test.id().split(".")[-1])
result = Result()
suite.run(result)
for test, _ in result.failures + result.errors:
    out["failed"].append(test.id().split(".")[-1] if hasattr(test, "id") else str(test))
out["run"] = result.testsRun
print(json.dumps(out))
"""


def task_acceptance_module(run: Path, task: dict) -> Optional[Path]:
    """The scenario's acceptance test module for a board task, or None if the scenario does not know it."""
    try:
        meta = json.loads((run / "meta.json").read_text())
        scenario = Path(meta.get("scenario_dir") or meta["kit"])
        specs = json.loads((scenario / "tasks.json").read_text())["tasks"]
        ids = json.loads((run / "task_ids.json").read_text())
    except (OSError, ValueError, KeyError):
        return None
    task_id = str(task.get("id") or "")
    key = next((k for k, full in ids.items() if task_id and str(full).startswith(task_id)), None)
    spec = next((t for t in specs if key and t.get("key") == key), None) or \
        next((t for t in specs if t.get("title") == task.get("title")), None)
    if not spec or not spec.get("hidden_test"):
        return None
    module = scenario / "hidden_tests" / spec["hidden_test"]
    return module if module.is_file() else None


def run_task_tests(repo: Path, module: Path, config: dict) -> dict:
    """Run one acceptance module against a scratch copy of the repository (never inside it)."""
    with tempfile.TemporaryDirectory(prefix="stop_gate_") as tmp:
        code = Path(tmp) / "code"
        shutil.copytree(str(repo), str(code), ignore=shutil.ignore_patterns(
            ".git", ".tirith", ".claude", ".env", "__pycache__", "*.pyc"))
        package = code / "gate_acceptance"
        package.mkdir()
        (package / "__init__.py").write_text("")
        shutil.copy(str(module), str(package / module.name))
        try:
            proc = subprocess.run(["python3", "-c", RUNNER, "gate_acceptance." + module.stem], cwd=str(code),
                                  capture_output=True, text=True, timeout=config["test_timeout_secs"],
                                  env={"PATH": "/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE": "1"})
            result = json.loads(proc.stdout.strip().splitlines()[-1])
        except subprocess.TimeoutExpired:
            result = {"run": 0, "passed": [], "failed": [], "import_error": "timed out after %ss"
                      % config["test_timeout_secs"]}
        except (ValueError, IndexError):
            result = {"run": 0, "passed": [], "failed": [], "import_error": "runner failed"}
    result["ok"] = not result.get("import_error") and not result["failed"] and result["run"] > 0
    return result


class PlaceholderGate:
    """Done iff the task's tests pass and every acceptance criterion is mentioned in the final message.

    Deterministic and deliberately crude: "mentioned" means a backticked
    identifier from the criterion appears in the message, or, for criteria
    without backticks, half of their longer words do. It cannot tell a
    claimed criterion from a met one. Which tests count is `gate_tests`
    (module docstring).
    """

    name = "placeholder"

    def evaluate(self, inp: GateInput) -> Verdict:
        scope = inp.config.get("gate_tests", "task")
        module = task_acceptance_module(inp.run or inp.repo.parent, inp.task) if scope == "task" else None
        suite = run_tests(inp.repo, inp.config)
        evidence = {"tests_scope": "task" if module else "suite", "suite_ok": suite["ok"]}
        if module:
            own = run_task_tests(inp.repo, module, inp.config)
            tests_ok = own["ok"]
            evidence.update(tests_passed=len(own["passed"]), tests_run=own["run"])
            if own.get("import_error"):
                evidence["import_error"] = str(own["import_error"])[:160]
        else:
            tests_ok = suite["ok"]
        criteria = acceptance_criteria(inp.task.get("description", ""))
        missing = [crit for crit in criteria if not mentioned(crit, inp.last_message or "")]
        evidence.update(tests_ok=tests_ok, criteria=len(criteria), missing=len(missing))
        if tests_ok and not missing:
            return Verdict("done", "stop gate (placeholder): %s pass, %d/%d criteria addressed"
                           % ("task acceptance tests" if module else "tests", len(criteria), len(criteria)),
                           evidence)
        if inp.continues >= inp.config["gate_max_continues"]:
            return Verdict("escalate", "stop gate (placeholder) still not satisfied after %d continues: %s"
                           % (inp.continues, ("task acceptance tests fail" if module else "tests fail")
                              if not tests_ok else "%d criteria unaddressed" % len(missing)), evidence)
        parts = []
        if not tests_ok and module:
            detail = ("the tests could not load the code (%s)" % str(own["import_error"])[:300]
                      if own.get("import_error") else
                      "%d of %d fail: %s" % (len(own["failed"]), own["run"], ", ".join(own["failed"][:20])))
            parts.append("The acceptance checks for this task run outside the repository, and %s. Only this "
                         "task's checks count: failures in other tasks' tests do not hold this task. The "
                         "acceptance criteria in the task description are what these checks test." % detail)
        elif not tests_ok:
            parts.append("`%s` does not pass in the repository. Its output ends with:\n%s"
                         % (" ".join(inp.config["test_command"]), suite["tail"]))
        if missing:
            parts.append("Your last message does not say how these acceptance criteria are met:\n%s"
                         % "\n".join("- " + crit for crit in missing))
        return Verdict("continue", "\n\n".join(parts), evidence)


GATES: Dict[str, Callable[[], object]] = {"placeholder": PlaceholderGate}


def load_gate(name: Optional[str]):
    try:
        return GATES[name or "placeholder"]()
    except KeyError:
        raise ValueError("unknown stop gate %r (known: %s)" % (name, ", ".join(sorted(GATES))))
