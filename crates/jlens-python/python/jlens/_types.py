"""The types of the values `Expression.evaluate_with_audit` takes and returns."""

from typing import Any, TypedDict

__all__ = ["AuditResult", "CaptureLimits", "ErrorInfo"]


class CaptureLimits(TypedDict, total=False):
    """Bounds on what an audit captures."""

    max_items: int
    """Items kept from an array (and fields from an object) in captured values. Default 10."""
    max_string: int
    """Characters kept from a string in captured values. Default 200."""
    max_depth: int
    """Nesting depth kept in captured values. Default 4."""
    max_rejected: int
    """Rejected items kept as samples per filter. Default 5."""
    max_nodes: int
    """Nodes recorded in the tree. Default 10000."""
    max_source: int
    """Characters of source text kept per node. Default 200."""


class ErrorInfo(TypedDict):
    """The fields of a `JlensError`, as an audit reports them."""

    code: str
    message: str
    position: int | None
    start: int | None
    end: int | None
    token: str | None
    value: Any


class AuditResult(TypedDict):
    """The result of `Expression.evaluate_with_audit`."""

    result: Any
    """The result; `None` when it is undefined or `null`, or when the evaluation failed."""
    error: ErrorInfo | None
    """The error, when the evaluation failed."""
    audit: dict[str, Any]
    """The audit tree, as `jlens eval --audit json` prints it: `header`, `root`, `nodeCount` and
    `truncated`. Its spans count code points."""
    explanation: dict[str, Any]
    """Why the result was reached: `result`, the result or the error as text, and `lines`."""
    explanation_text: str
    """`explanation` as text, as printed by `jlens eval --explain`."""
