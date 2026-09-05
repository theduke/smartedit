from typing import Protocol, overload


class Renderer(Protocol):
    def render(self, value: str) -> str: ...


@overload
def parse(value: str) -> str: ...


@overload
def parse(value: bytes) -> bytes: ...
