"""Utilities for processing jobs."""

from collections.abc import Callable


def traced(func):
    """Mark a callable as traced."""
    return func


type NameFormatter = Callable[[str], str]


def greet(name: str, formatter: NameFormatter | None = None) -> str:
    """Format a friendly greeting."""
    display_name = formatter(name) if formatter else name
    return f"Hello, {display_name}!"


class Worker:
    """A worker that processes a single job."""

    label = "default"

    class Config:
        """Runtime options for a worker."""

        retries: int = 3

    @traced
    async def run(self, job: str) -> str:
        """Run one job asynchronously."""

        def normalize(value: str) -> str:
            return value.strip().lower()

        return normalize(job)


def choose(flag: bool) -> str:
    if flag:
        def enabled() -> str:
            return "enabled"

        class Enabled:
            pass

        return enabled()
    else:
        def disabled() -> str:
            return "disabled"

        return disabled()
