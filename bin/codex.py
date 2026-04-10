#!/usr/bin/env python3

# Codex session utilities for analyzing rollout logs stored under `~/.codex`.
#
# Subcommands:
# - `measure-patch-usage`: measure how much rollout text and token usage is
#   associated with patch generation.
# - `session-analyze`: show token totals and runtime for one session.
# - `session-diff`: compare token totals and runtime between two sessions.
#
# Measurement semantics for `measure-patch-usage`:
# - The text-size denominator is the raw byte size of the full rollout JSONL
#   session file as stored on disk.
# - The text-size numerator is the raw byte size of JSONL records for
#   `apply_patch` tool calls plus `patch_apply_end` events.
# - Token counters come from Codex `event_msg` records of type `token_count`,
#   using cumulative totals from `total_token_usage`.
# - Patch-attributed tokens are estimated by assigning the next token-count
#   delta after an `apply_patch` tool call to patching activity.
# - Malformed or non-UTF-8 lines are skipped so a few bad records do not abort
#   the full scan.

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path


PATCH_TOOL_NAMES = {"apply_patch"}
SESSION_SELECTOR_LIMIT = 20


@dataclass
class TokenUsage:
    input_tokens: int = 0
    output_tokens: int = 0
    reasoning_tokens: int = 0

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens + self.reasoning_tokens

    def delta_from(self, previous: "TokenUsage") -> "TokenUsage":
        return TokenUsage(
            input_tokens=max(0, self.input_tokens - previous.input_tokens),
            output_tokens=max(0, self.output_tokens - previous.output_tokens),
            reasoning_tokens=max(0, self.reasoning_tokens - previous.reasoning_tokens),
        )


@dataclass
class SessionMetadata:
    path: Path
    session_id: str
    session_name: str
    started_at: str
    model: str


@dataclass
class SessionStats:
    path: Path
    session_id: str
    session_name: str
    started_at: str
    model: str
    total_bytes: int
    patch_bytes: int
    total_lines: int
    patch_records: int
    malformed_lines: int
    total_tokens: TokenUsage
    patch_tokens: TokenUsage
    runtime_seconds: float | None

    @property
    def patch_pct(self) -> float:
        if self.total_bytes == 0:
            return 0.0
        return (self.patch_bytes / self.total_bytes) * 100.0

    def token_pct(self, patch_tokens: int, total_tokens: int) -> float:
        if total_tokens == 0:
            return 0.0
        return (patch_tokens / total_tokens) * 100.0

    @property
    def patch_input_pct(self) -> float:
        return self.token_pct(
            self.patch_tokens.input_tokens, self.total_tokens.input_tokens
        )

    @property
    def patch_output_pct(self) -> float:
        return self.token_pct(
            self.patch_tokens.output_tokens, self.total_tokens.output_tokens
        )

    @property
    def patch_reasoning_pct(self) -> float:
        return self.token_pct(
            self.patch_tokens.reasoning_tokens, self.total_tokens.reasoning_tokens
        )


def summarize_session_name(text: str, limit: int = 72) -> str:
    compact = " ".join(text.split())
    if not compact:
        return "-"
    if len(compact) <= limit:
        return compact
    return compact[: limit - 3] + "..."


def format_int(value: int) -> str:
    return f"{value:,}"


def format_pct(value: float) -> str:
    return f"{value:.2f}%"


def format_runtime_seconds(value: float | None) -> str:
    if value is None:
        return "n/a"

    duration = timedelta(seconds=max(0.0, value))
    total_seconds = duration.total_seconds()
    hours, remainder = divmod(int(total_seconds), 3600)
    minutes, seconds = divmod(remainder, 60)
    fractional = total_seconds - int(total_seconds)

    if hours:
        return f"{hours}h {minutes}m {seconds + fractional:05.2f}s"
    if minutes:
        return f"{minutes}m {seconds + fractional:05.2f}s"
    return f"{total_seconds:.2f}s"


def format_runtime_delta(value: float | None) -> str:
    if value is None:
        return "n/a"
    sign = "+" if value >= 0 else "-"
    return f"{sign}{format_runtime_seconds(abs(value))}"


def truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return value[: limit - 3] + "..."


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Codex rollout analysis CLI")
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path.home() / ".codex",
        help="Codex home directory. Default: ~/.codex",
    )
    parser.add_argument(
        "--include-archived",
        action="store_true",
        help="Also scan ~/.codex/archived_sessions/*.jsonl",
    )

    subparsers = parser.add_subparsers(dest="command", required=True)

    measure = subparsers.add_parser(
        "measure-patch-usage",
        help="Measure patch-related text and token usage across sessions",
    )
    measure.add_argument(
        "--patched-only",
        action="store_true",
        help="Only print sessions that contain patch records.",
    )
    measure.add_argument(
        "--sort",
        choices=("patch-pct", "patch-bytes", "total-bytes", "date"),
        default="patch-pct",
        help="Sort order for the session table. Default: patch-pct",
    )
    measure.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Only print the first N rows after sorting. Default: all rows",
    )
    measure.add_argument(
        "--progress-every",
        type=int,
        default=25,
        help="Print progress every N sessions. Default: 25",
    )

    analyze = subparsers.add_parser(
        "session-analyze",
        help="Show token totals and runtime for a session",
    )
    analyze.add_argument("session_id", nargs="?", help="Session id or unique prefix")

    diff = subparsers.add_parser(
        "session-diff",
        help="Compare token totals and runtime between two sessions",
    )
    diff.add_argument("session_a", nargs="?", help="First session id or unique prefix")
    diff.add_argument("session_b", nargs="?", help="Second session id or unique prefix")

    return parser.parse_args()


def discover_session_files(codex_home: Path, include_archived: bool) -> list[Path]:
    files = sorted((codex_home / "sessions").rglob("rollout-*.jsonl"))
    if include_archived:
        files.extend(sorted((codex_home / "archived_sessions").glob("rollout-*.jsonl")))
    return files


def fallback_session_id(path: Path) -> str:
    stem = path.stem
    parts = stem.split("-")
    if len(parts) >= 2:
        return parts[-1]
    return stem


def extract_session_name_from_user_content(content: object) -> str:
    if not isinstance(content, list):
        return ""

    candidates: list[str] = []
    for item in content:
        if not isinstance(item, dict):
            continue
        text = item.get("text")
        if not isinstance(text, str):
            continue

        stripped = text.strip()
        if not stripped:
            continue
        if stripped.startswith("<environment_context>"):
            continue
        if stripped.startswith("# AGENTS.md instructions"):
            continue
        if stripped.startswith("<user_instructions>"):
            continue
        if stripped.startswith("<workspace_info>"):
            continue

        candidates.append(stripped)

    if not candidates:
        return ""

    return summarize_session_name(candidates[-1])


def extract_total_token_usage(record: object) -> TokenUsage | None:
    if not isinstance(record, dict) or record.get("type") != "event_msg":
        return None

    payload = record.get("payload")
    if not isinstance(payload, dict) or payload.get("type") != "token_count":
        return None

    info = payload.get("info")
    if not isinstance(info, dict):
        return None

    totals = info.get("total_token_usage")
    if not isinstance(totals, dict):
        return None

    return TokenUsage(
        input_tokens=int(totals.get("input_tokens") or 0),
        output_tokens=int(totals.get("output_tokens") or 0),
        reasoning_tokens=int(totals.get("reasoning_output_tokens") or 0),
    )


def parse_iso8601_timestamp(value: object) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None

    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def extract_record_timestamp(record: object) -> datetime | None:
    if not isinstance(record, dict):
        return None
    return parse_iso8601_timestamp(record.get("timestamp"))


def is_patch_record(record: object) -> bool:
    if not isinstance(record, dict):
        return False

    payload = record.get("payload")
    if not isinstance(payload, dict):
        return False

    if record.get("type") == "response_item":
        return (
            payload.get("type") == "custom_tool_call"
            and payload.get("name") in PATCH_TOOL_NAMES
        )

    if record.get("type") == "event_msg":
        return payload.get("type") == "patch_apply_end"

    return False


def is_patch_generation_record(record: object) -> bool:
    if not isinstance(record, dict):
        return False

    payload = record.get("payload")
    if not isinstance(payload, dict):
        return False

    return (
        record.get("type") == "response_item"
        and payload.get("type") == "custom_tool_call"
        and payload.get("name") in PATCH_TOOL_NAMES
    )


def load_session_catalog(
    codex_home: Path,
    include_archived: bool,
    show_progress: bool = False,
    progress_every: int = 200,
) -> list[SessionMetadata]:
    session_files = discover_session_files(codex_home, include_archived)
    if not session_files:
        return []

    if show_progress:
        print(
            f"Indexing {len(session_files)} sessions under {codex_home}...",
            file=sys.stderr,
            flush=True,
        )

    catalog: list[SessionMetadata] = []
    for idx, path in enumerate(session_files, start=1):
        catalog.append(read_session_metadata(path))
        if show_progress and (
            idx == 1 or idx == len(session_files) or idx % max(progress_every, 1) == 0
        ):
            print(
                f"[{idx}/{len(session_files)}] indexed {path.name}",
                file=sys.stderr,
                flush=True,
            )

    return sorted(catalog, key=lambda item: (item.started_at, item.session_id), reverse=True)


def read_session_metadata(path: Path) -> SessionMetadata:
    session_id = fallback_session_id(path)
    session_name = ""
    started_at = ""
    model = ""

    with path.open("rb") as handle:
        for raw_line in handle:
            if not raw_line.strip():
                continue

            try:
                record = json.loads(raw_line.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue

            if isinstance(record, dict) and record.get("type") == "session_meta":
                payload = record.get("payload")
                if isinstance(payload, dict):
                    session_id = str(payload.get("id") or session_id)
                    started_at = str(payload.get("timestamp") or started_at)

            if isinstance(record, dict) and record.get("type") == "turn_context" and not model:
                payload = record.get("payload")
                if isinstance(payload, dict):
                    model = str(payload.get("model") or model)

            if (
                isinstance(record, dict)
                and record.get("type") == "response_item"
                and not session_name
            ):
                payload = record.get("payload")
                if (
                    isinstance(payload, dict)
                    and payload.get("type") == "message"
                    and payload.get("role") == "user"
                ):
                    session_name = extract_session_name_from_user_content(
                        payload.get("content")
                    )

            if session_id and session_name and started_at and model:
                break

    return SessionMetadata(
        path=path,
        session_id=session_id,
        session_name=session_name or path.name,
        started_at=started_at or "-",
        model=model or "-",
    )


def resolve_session_metadata(
    session_query: str | None,
    catalog: list[SessionMetadata],
    prompt: str,
    exclude_ids: set[str] | None = None,
) -> SessionMetadata:
    exclude_ids = exclude_ids or set()
    candidates = [item for item in catalog if item.session_id not in exclude_ids]

    if session_query:
        exact_matches = [item for item in candidates if item.session_id == session_query]
        if len(exact_matches) == 1:
            return exact_matches[0]

        prefix_matches = [
            item for item in candidates if item.session_id.startswith(session_query)
        ]
        if len(prefix_matches) == 1:
            return prefix_matches[0]
        if len(prefix_matches) > 1:
            joined = ", ".join(item.session_id for item in prefix_matches[:8])
            raise SystemExit(
                f"session id `{session_query}` is ambiguous; matches: {joined}"
            )
        raise SystemExit(f"session id `{session_query}` not found")

    return prompt_for_session_selection(candidates, prompt)


def prompt_for_session_selection(
    catalog: list[SessionMetadata],
    prompt: str,
) -> SessionMetadata:
    if not sys.stdin.isatty():
        raise SystemExit(f"{prompt}: session id is required when stdin is not interactive")

    filtered = catalog
    query = ""
    while True:
        if query:
            lowered = query.lower()
            filtered = [
                item
                for item in catalog
                if lowered in item.session_name.lower()
                or lowered in item.session_id.lower()
                or lowered in item.model.lower()
            ]
        else:
            filtered = catalog

        if not filtered:
            print("No sessions matched that filter.", file=sys.stderr)
        else:
            print(f"\n{prompt}", file=sys.stderr)
            for idx, item in enumerate(filtered[:SESSION_SELECTOR_LIMIT], start=1):
                print(
                    f"{idx:>2}. {item.started_at}  {truncate(item.session_name, 72)}",
                    file=sys.stderr,
                )
            if len(filtered) > SESSION_SELECTOR_LIMIT:
                print(
                    f"... showing first {SESSION_SELECTOR_LIMIT} of {len(filtered)} matches",
                    file=sys.stderr,
                )

        choice = input(
            "Enter number, a new filter string, or 'q' to quit"
            + (" [blank=recent]" if not query else "")
            + ": "
        ).strip()

        if choice.lower() == "q":
            raise SystemExit("selection cancelled")
        if not choice:
            query = ""
            continue
        if choice.isdigit():
            index = int(choice) - 1
            if 0 <= index < min(len(filtered), SESSION_SELECTOR_LIMIT):
                return filtered[index]
            print("Selection out of range.", file=sys.stderr)
            continue

        query = choice


def analyze_session(path: Path) -> SessionStats:
    metadata = read_session_metadata(path)
    total_bytes = 0
    patch_bytes = 0
    total_lines = 0
    patch_records = 0
    malformed_lines = 0
    total_token_usage = TokenUsage()
    previous_token_usage = TokenUsage()
    patch_token_usage = TokenUsage()
    saw_patch_generation_since_last_token_count = False
    started_at_dt = parse_iso8601_timestamp(metadata.started_at)
    last_record_timestamp: datetime | None = None

    with path.open("rb") as handle:
        for raw_line in handle:
            total_lines += 1
            total_bytes += len(raw_line)

            if not raw_line.strip():
                continue

            try:
                record = json.loads(raw_line.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                malformed_lines += 1
                continue

            record_timestamp = extract_record_timestamp(record)
            if record_timestamp is not None:
                last_record_timestamp = record_timestamp

            token_usage = extract_total_token_usage(record)
            if token_usage is not None:
                delta = token_usage.delta_from(previous_token_usage)
                total_token_usage = token_usage
                if saw_patch_generation_since_last_token_count:
                    patch_token_usage.input_tokens += delta.input_tokens
                    patch_token_usage.output_tokens += delta.output_tokens
                    patch_token_usage.reasoning_tokens += delta.reasoning_tokens
                previous_token_usage = token_usage
                saw_patch_generation_since_last_token_count = False

            if is_patch_generation_record(record):
                saw_patch_generation_since_last_token_count = True

            if is_patch_record(record):
                patch_bytes += len(raw_line)
                patch_records += 1

    runtime_seconds = None
    if started_at_dt is not None and last_record_timestamp is not None:
        runtime_seconds = max(
            0.0, (last_record_timestamp - started_at_dt).total_seconds()
        )

    return SessionStats(
        path=path,
        session_id=metadata.session_id,
        session_name=metadata.session_name,
        started_at=metadata.started_at,
        model=metadata.model,
        total_bytes=total_bytes,
        patch_bytes=patch_bytes,
        total_lines=total_lines,
        patch_records=patch_records,
        malformed_lines=malformed_lines,
        total_tokens=total_token_usage,
        patch_tokens=patch_token_usage,
        runtime_seconds=runtime_seconds,
    )


def sort_sessions(items: list[SessionStats], sort_key: str) -> list[SessionStats]:
    if sort_key == "date":
        return sorted(
            items,
            key=lambda item: (item.started_at, item.session_id),
            reverse=True,
        )
    if sort_key == "patch-bytes":
        return sorted(
            items,
            key=lambda item: (item.patch_bytes, item.patch_pct, item.total_bytes),
            reverse=True,
        )
    if sort_key == "total-bytes":
        return sorted(
            items,
            key=lambda item: (item.total_bytes, item.patch_bytes),
            reverse=True,
        )
    return sorted(
        items,
        key=lambda item: (item.patch_pct, item.patch_bytes, item.total_bytes),
        reverse=True,
    )


def print_patch_progress(
    processed: int, total: int, current: Path, aggregate: list[SessionStats]
) -> None:
    total_bytes = sum(item.total_bytes for item in aggregate)
    patch_bytes = sum(item.patch_bytes for item in aggregate)
    pct = (patch_bytes / total_bytes * 100.0) if total_bytes else 0.0
    print(
        f"[{processed}/{total}] {current.name} | "
        f"patch so far: {format_int(patch_bytes)} / {format_int(total_bytes)} bytes ({pct:.2f}%)",
        file=sys.stderr,
        flush=True,
    )


def print_measure_summary(codex_home: Path, stats: list[SessionStats]) -> None:
    total_sessions = len(stats)
    patched_sessions = sum(1 for item in stats if item.patch_bytes > 0)
    total_bytes = sum(item.total_bytes for item in stats)
    patch_bytes = sum(item.patch_bytes for item in stats)
    malformed_lines = sum(item.malformed_lines for item in stats)

    total_input_tokens = sum(item.total_tokens.input_tokens for item in stats)
    total_output_tokens = sum(item.total_tokens.output_tokens for item in stats)
    total_reasoning_tokens = sum(item.total_tokens.reasoning_tokens for item in stats)
    patch_input_tokens = sum(item.patch_tokens.input_tokens for item in stats)
    patch_output_tokens = sum(item.patch_tokens.output_tokens for item in stats)
    patch_reasoning_tokens = sum(item.patch_tokens.reasoning_tokens for item in stats)

    print(f"Codex home: {codex_home}")
    print(f"Sessions scanned: {total_sessions}")
    print(f"Sessions with patching: {patched_sessions}")
    print(
        f"Patch text size: {format_int(patch_bytes)} / {format_int(total_bytes)} bytes "
        f"({format_pct((patch_bytes / total_bytes * 100.0) if total_bytes else 0.0)})"
    )
    print(
        f"Input tokens: {format_int(patch_input_tokens)} / {format_int(total_input_tokens)} "
        f"({format_pct((patch_input_tokens / total_input_tokens * 100.0) if total_input_tokens else 0.0)})"
    )
    print(
        f"Output tokens: {format_int(patch_output_tokens)} / {format_int(total_output_tokens)} "
        f"({format_pct((patch_output_tokens / total_output_tokens * 100.0) if total_output_tokens else 0.0)})"
    )
    print(
        f"Thinking tokens: {format_int(patch_reasoning_tokens)} / {format_int(total_reasoning_tokens)} "
        f"({format_pct((patch_reasoning_tokens / total_reasoning_tokens * 100.0) if total_reasoning_tokens else 0.0)})"
    )
    print(f"Malformed JSONL lines skipped: {format_int(malformed_lines)}")
    print()


def print_measure_table(stats: list[SessionStats]) -> None:
    headers = [
        "session",
        "name",
        "started_at",
        "model",
        "patch_bytes",
        "total_bytes",
        "patch_pct",
        "input_tokens",
        "input_patch",
        "output_tokens",
        "output_patch",
        "thinking_tokens",
        "thinking_patch",
        "patch_records",
    ]

    rows = []
    for item in stats:
        rows.append(
            [
                item.session_id,
                item.session_name,
                item.started_at,
                item.model,
                format_int(item.patch_bytes),
                format_int(item.total_bytes),
                format_pct(item.patch_pct),
                format_int(item.total_tokens.input_tokens),
                f"{format_int(item.patch_tokens.input_tokens)} ({format_pct(item.patch_input_pct)})",
                format_int(item.total_tokens.output_tokens),
                f"{format_int(item.patch_tokens.output_tokens)} ({format_pct(item.patch_output_pct)})",
                format_int(item.total_tokens.reasoning_tokens),
                f"{format_int(item.patch_tokens.reasoning_tokens)} ({format_pct(item.patch_reasoning_pct)})",
                str(item.patch_records),
            ]
        )

    print_table(headers, rows)


def print_table(headers: list[str], rows: list[list[str]]) -> None:
    if not rows:
        print("(no rows)")
        return

    widths = [len(header) for header in headers]
    for row in rows:
        for idx, cell in enumerate(row):
            widths[idx] = max(widths[idx], len(cell))

    print("  ".join(header.ljust(widths[idx]) for idx, header in enumerate(headers)))
    print("  ".join("-" * width for width in widths))
    for row in rows:
        print("  ".join(cell.ljust(widths[idx]) for idx, cell in enumerate(row)))


def print_session_analysis(stats: SessionStats) -> None:
    print(f"Session: {stats.session_id}")
    print(f"Name: {stats.session_name}")
    print(f"Started: {stats.started_at}")
    print(f"Model: {stats.model}")
    print(f"Runtime: {format_runtime_seconds(stats.runtime_seconds)}")
    print()
    print_table(
        ["metric", "tokens"],
        [
            ["input", format_int(stats.total_tokens.input_tokens)],
            ["thinking", format_int(stats.total_tokens.reasoning_tokens)],
            ["output", format_int(stats.total_tokens.output_tokens)],
            ["total", format_int(stats.total_tokens.total_tokens)],
        ],
    )


def format_delta(value: int) -> str:
    return f"{value:+,}"


def format_delta_pct(base: float, other: float) -> str:
    if base == 0:
        return "n/a" if other != 0 else "0.00%"
    return f"{((other - base) / base) * 100.0:+.2f}%"


def print_session_diff(left: SessionStats, right: SessionStats) -> None:
    print(f"A: {left.session_id}  {left.started_at}  {left.session_name}")
    print(f"B: {right.session_id}  {right.started_at}  {right.session_name}")
    print()
    print_table(
        ["metric", "A", "B", "delta", "delta_pct"],
        [
            [
                "runtime",
                format_runtime_seconds(left.runtime_seconds),
                format_runtime_seconds(right.runtime_seconds),
                format_runtime_delta(
                    None
                    if left.runtime_seconds is None or right.runtime_seconds is None
                    else right.runtime_seconds - left.runtime_seconds
                ),
                format_delta_pct(
                    left.runtime_seconds or 0.0,
                    right.runtime_seconds or 0.0,
                )
                if left.runtime_seconds is not None and right.runtime_seconds is not None
                else "n/a",
            ],
            [
                "input",
                format_int(left.total_tokens.input_tokens),
                format_int(right.total_tokens.input_tokens),
                format_delta(right.total_tokens.input_tokens - left.total_tokens.input_tokens),
                format_delta_pct(
                    left.total_tokens.input_tokens, right.total_tokens.input_tokens
                ),
            ],
            [
                "thinking",
                format_int(left.total_tokens.reasoning_tokens),
                format_int(right.total_tokens.reasoning_tokens),
                format_delta(
                    right.total_tokens.reasoning_tokens
                    - left.total_tokens.reasoning_tokens
                ),
                format_delta_pct(
                    left.total_tokens.reasoning_tokens,
                    right.total_tokens.reasoning_tokens,
                ),
            ],
            [
                "output",
                format_int(left.total_tokens.output_tokens),
                format_int(right.total_tokens.output_tokens),
                format_delta(
                    right.total_tokens.output_tokens - left.total_tokens.output_tokens
                ),
                format_delta_pct(
                    left.total_tokens.output_tokens, right.total_tokens.output_tokens
                ),
            ],
            [
                "total",
                format_int(left.total_tokens.total_tokens),
                format_int(right.total_tokens.total_tokens),
                format_delta(right.total_tokens.total_tokens - left.total_tokens.total_tokens),
                format_delta_pct(
                    left.total_tokens.total_tokens, right.total_tokens.total_tokens
                ),
            ],
        ],
    )


def run_measure_patch_usage(args: argparse.Namespace) -> int:
    codex_home = args.codex_home.expanduser().resolve()
    session_files = discover_session_files(codex_home, args.include_archived)
    if not session_files:
        print(f"No rollout session files found under {codex_home}", file=sys.stderr)
        return 1

    print(
        f"Scanning {len(session_files)} session files under {codex_home}...",
        file=sys.stderr,
        flush=True,
    )

    stats: list[SessionStats] = []
    for idx, path in enumerate(session_files, start=1):
        stats.append(analyze_session(path))
        if idx == 1 or idx == len(session_files) or idx % max(args.progress_every, 1) == 0:
            print_patch_progress(idx, len(session_files), path, stats)

    filtered = stats
    if args.patched_only:
        filtered = [item for item in filtered if item.patch_bytes > 0]
    filtered = sort_sessions(filtered, args.sort)
    if args.limit > 0:
        filtered = filtered[: args.limit]

    print_measure_summary(codex_home, stats)
    print_measure_table(filtered)
    return 0


def run_session_analyze(args: argparse.Namespace) -> int:
    codex_home = args.codex_home.expanduser().resolve()
    catalog = load_session_catalog(codex_home, args.include_archived, show_progress=True)
    if not catalog:
        print(f"No rollout session files found under {codex_home}", file=sys.stderr)
        return 1

    selected = resolve_session_metadata(args.session_id, catalog, "Select a session")
    stats = analyze_session(selected.path)
    print_session_analysis(stats)
    return 0


def run_session_diff(args: argparse.Namespace) -> int:
    codex_home = args.codex_home.expanduser().resolve()
    catalog = load_session_catalog(codex_home, args.include_archived, show_progress=True)
    if not catalog:
        print(f"No rollout session files found under {codex_home}", file=sys.stderr)
        return 1

    left_meta = resolve_session_metadata(args.session_a, catalog, "Select session A")
    right_meta = resolve_session_metadata(
        args.session_b,
        catalog,
        "Select session B",
        exclude_ids={left_meta.session_id},
    )

    left = analyze_session(left_meta.path)
    right = analyze_session(right_meta.path)
    print_session_diff(left, right)
    return 0


def main() -> int:
    args = parse_args()
    if args.command == "measure-patch-usage":
        return run_measure_patch_usage(args)
    if args.command == "session-analyze":
        return run_session_analyze(args)
    if args.command == "session-diff":
        return run_session_diff(args)
    raise SystemExit(f"unknown command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
