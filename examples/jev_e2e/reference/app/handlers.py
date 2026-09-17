"""The demo application's routes: a tiny in-memory item store."""

import json
from typing import Dict, List

from app.pipeline import Context, HttpError, Request, Response
from app.router import Router

_ITEMS: Dict[str, Dict[str, object]] = {
    "1": {"id": "1", "name": "anvil", "stock": 3},
    "2": {"id": "2", "name": "bellows", "stock": 0},
    "3": {"id": "3", "name": "crucible", "stock": 12},
}


def reset_items() -> None:
    """Restore the seed data (tests call this in setUp)."""
    _ITEMS.clear()
    _ITEMS.update(
        {
            "1": {"id": "1", "name": "anvil", "stock": 3},
            "2": {"id": "2", "name": "bellows", "stock": 0},
            "3": {"id": "3", "name": "crucible", "stock": 12},
        }
    )


def health(request: Request, ctx: Context) -> Response:
    return Response.text("ok")


def public_status(request: Request, ctx: Context) -> Response:
    return Response.json({"status": "up", "items": len(_ITEMS)})


def list_items(request: Request, ctx: Context) -> Response:
    rows: List[Dict[str, object]] = sorted(_ITEMS.values(), key=lambda row: str(row["id"]))
    if request.query.get("in_stock") == "1":
        rows = [row for row in rows if int(str(row["stock"])) > 0]
    return Response.json({"items": rows})


def get_item(request: Request, ctx: Context) -> Response:
    item = _ITEMS.get(ctx.params["item_id"])
    if item is None:
        raise HttpError(404, "no such item")
    return Response.json(item)


def create_item(request: Request, ctx: Context) -> Response:
    try:
        payload = json.loads(request.body.decode("utf-8") or "{}")
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise HttpError(400, "body must be JSON")
    name = payload.get("name") if isinstance(payload, dict) else None
    if not isinstance(name, str) or not name:
        raise HttpError(400, "name is required")
    item_id = str(max((int(k) for k in _ITEMS), default=0) + 1)
    item = {"id": item_id, "name": name, "stock": int(payload.get("stock", 0))}
    _ITEMS[item_id] = item
    return Response.json(item, status=201)


def report(request: Request, ctx: Context) -> Response:
    """A deliberately large, repetitive text body."""
    lines = ["%s,%s,%s" % (row["id"], row["name"], row["stock"]) for row in _ITEMS.values()]
    return Response.text("\n".join(["id,name,stock"] + lines * 200))


def default_router() -> Router:
    router = Router()
    router.add("GET", "/health", health, "health")
    router.add("GET", "/public/status", public_status, "public_status")
    router.add("GET", "/items", list_items, "list_items")
    router.add("POST", "/items", create_item, "create_item")
    router.add("GET", "/items/{item_id}", get_item, "get_item")
    router.add("GET", "/report", report, "report")
    return router
