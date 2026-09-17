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
"""

import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Dict, List, Optional


@dataclass
class GateInput:
    """What the gate sees at a stop."""

    task: dict                   # Tirith task row: id, title, description, paths
    last_message: str            # last_assistant_message from the Stop input
    repo: Path                   # repository root to test
    agent: str
    config: dict
    continues: int = 0           # continue verdicts already given for this task


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


class PlaceholderGate:
    """Done iff the test command passes and every acceptance criterion is mentioned in the final message.

    Deterministic and deliberately crude: "mentioned" means a backticked
    identifier from the criterion appears in the message, or, for criteria
    without backticks, half of their longer words do. It cannot tell a
    claimed criterion from a met one.
    """

    name = "placeholder"

    def evaluate(self, inp: GateInput) -> Verdict:
        tests = run_tests(inp.repo, inp.config)
        criteria = acceptance_criteria(inp.task.get("description", ""))
        missing = [crit for crit in criteria if not mentioned(crit, inp.last_message or "")]
        evidence = {"tests_ok": tests["ok"], "criteria": len(criteria), "missing": len(missing)}
        if tests["ok"] and not missing:
            return Verdict("done", "stop gate (placeholder): tests pass, %d/%d criteria addressed"
                           % (len(criteria), len(criteria)), evidence)
        if inp.continues >= inp.config["gate_max_continues"]:
            return Verdict("escalate", "stop gate (placeholder) still not satisfied after %d continues: %s"
                           % (inp.continues, "tests fail" if not tests["ok"] else "%d criteria unaddressed"
                              % len(missing)), evidence)
        parts = []
        if not tests["ok"]:
            parts.append("`%s` does not pass in the repository. Its output ends with:\n%s"
                         % (" ".join(inp.config["test_command"]), tests["tail"]))
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
