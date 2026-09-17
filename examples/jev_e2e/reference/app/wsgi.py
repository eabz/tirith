"""WSGI adapter: ``gunicorn 'app.wsgi:make_wsgi_app()'``."""

from typing import Callable, Iterable, List, Mapping, Optional, Tuple
from urllib.parse import parse_qsl

from app.config import Config
from app.pipeline import Headers, Pipeline, Request
from app.router import build_pipeline

StartResponse = Callable[[str, List[Tuple[str, str]]], object]


def request_from_environ(environ: Mapping[str, object]) -> Request:
    headers = Headers()
    for key, value in environ.items():
        if key.startswith("HTTP_"):
            headers[key[5:].replace("_", "-").title()] = str(value)
    for key in ("CONTENT_TYPE", "CONTENT_LENGTH"):
        if environ.get(key):
            headers[key.replace("_", "-").title()] = str(environ[key])
    body = b""
    length = int(str(environ.get("CONTENT_LENGTH") or 0) or 0)
    stream = environ.get("wsgi.input")
    if length and stream is not None:
        body = stream.read(length)  # type: ignore[attr-defined]
    return Request(
        method=str(environ.get("REQUEST_METHOD", "GET")).upper(),
        path=str(environ.get("PATH_INFO", "/")) or "/",
        headers=headers,
        body=body,
        query=dict(parse_qsl(str(environ.get("QUERY_STRING", "")))),
        client_ip=str(environ.get("REMOTE_ADDR", "127.0.0.1")),
    )


def make_wsgi_app(pipeline: Optional[Pipeline] = None):
    pipeline = pipeline or build_pipeline(Config.from_env())

    def application(environ: Mapping[str, object], start_response: StartResponse) -> Iterable[bytes]:
        response = pipeline.dispatch(request_from_environ(environ))
        start_response("%d %s" % (response.status, response.reason), response.headers.items())
        return [response.body]

    return application

