from datetime import datetime
from typing import Any, final

from typing_extensions import Self

__all__ = ["Document", "Expression", "JlensError", "__version__"]

__version__: str

@final
class Expression:
    """A compiled JSONata expression. Compile once, then evaluate against any number of
    documents, from any thread."""

    def __new__(cls, source: str) -> Self:
        """Compiles a JSONata expression. Raises a `JlensError` on syntax errors."""

    @property
    def source(self) -> str:
        """The source text of the expression."""

    def evaluate(
        self,
        input: Any = None,
        bindings: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> Any:
        """Evaluates the expression against `input`: `dict`, `list`, `tuple`, `str`, `int`,
        `float`, `bool` and `None`, read one level at a time as the expression reaches it.
        `None` means no input document. Returns `None` for an undefined result or `null`.

        `bindings` maps names to values: `{"limit": 10}` defines `$limit`. The options limit the
        evaluation: `timeout` in seconds, `max_depth`, `max_string_length` (bytes),
        `max_array_length` (items) and `max_result_size` (bytes of JSON); `now` fixes the clock
        (milliseconds since the epoch, a `datetime` or an ISO 8601 timestamp) and `seed`
        `$random()`. Raises a `JlensError` when the evaluation fails."""

    def evaluate_json(
        self,
        input: str | None = None,
        bindings: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> str | None:
        """Like `evaluate`, with the input document and the result as JSON text: `None` for an
        undefined result, `"null"` for `null`. Other Python threads run during the
        evaluation."""

    def evaluate_document(
        self,
        document: Document,
        bindings: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> Any:
        """Evaluates the expression against a `Document` converted beforehand. Other Python
        threads run during the evaluation."""

@final
class Document:
    """A JSON document converted once, for evaluating several expressions (or one several
    times) against it. It can be shared between threads."""

    def __new__(cls, value: Any) -> Self:
        """Converts a Python value, with the rules of `Expression.evaluate`'s input."""

    @staticmethod
    def from_json(text: str) -> Document:
        """Parses JSON text."""

class JlensError(Exception):
    """A JSONata error. Offsets count code points, the unit of `str`."""

    code: str
    """The JSONata error code, such as `S0201` or `T2001`."""
    position: int | None
    """Where the error was raised in the expression."""
    start: int | None
    """The start of the part of the expression the error is about."""
    end: int | None
    """The end of that part."""
    token: str | None
    """The token the error is about."""
    value: Any
    """The value the error is about, or `None`."""
