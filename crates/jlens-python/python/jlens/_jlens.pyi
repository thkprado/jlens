from collections.abc import Callable
from datetime import datetime
from typing import Any, Literal, final

from typing_extensions import Self

from ._types import AuditResult, CaptureLimits, Read, Reads

__all__ = [
    "Document",
    "Expression",
    "Function",
    "JlensError",
    "__version__",
    "union_reads",
]

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
        max_steps: int | None = None,
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
        evaluation: `timeout` in seconds, `max_depth`, `max_steps`, `max_string_length`
        (bytes), `max_array_length` (items) and `max_result_size` (bytes of JSON); `now` fixes
        the clock (milliseconds since the epoch, a `datetime` or an ISO 8601 timestamp) and
        `seed` `$random()`. Raises a `JlensError` when the evaluation fails."""

    def evaluate_json(
        self,
        input: str | None = None,
        bindings: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_steps: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> str | None:
        """Like `evaluate`, with the input document and the result as JSON text: `None` for an
        undefined result, `"null"` for `null`. The fields of the input that the expression does
        not read (`reads()`) are skipped while parsing, checked as JSON syntax only. Other
        Python threads run during the evaluation."""

    def evaluate_document(
        self,
        document: Document,
        bindings: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_steps: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> Any:
        """Evaluates the expression against a `Document` converted beforehand. Other Python
        threads run during the evaluation."""

    def evaluate_with_audit(
        self,
        input: Any = None,
        bindings: dict[str, Any] | None = None,
        *,
        level: Literal["off", "decisions", "full"] | None = None,
        limits: CaptureLimits | None = None,
        timeout: float | None = None,
        max_depth: int | None = None,
        max_steps: int | None = None,
        max_string_length: int | None = None,
        max_array_length: int | None = None,
        max_result_size: int | None = None,
        now: int | float | datetime | str | None = None,
        seed: int | None = None,
    ) -> AuditResult:
        """Evaluates the expression and records how the result was reached: at `level`
        `"decisions"` (the default) the nodes that decide something, at `"full"` every node, at
        `"off"` none. The input is converted whole first. Evaluation errors are reported in the
        result's `error`, not raised; an exception of a host function is reported with its
        `code` attribute (or the name of its type) and its text."""

    def reads(self, bindings: dict[str, Any] | None = None) -> Reads:
        """The fields of the input and of the bindings the expression reads: an input cut down
        to them gives the same result. The functions in `bindings`, and those registered on the
        expression, read only their arguments; a call to a function it does not know reads its
        context whole."""

    def register_function(
        self,
        name: str,
        implementation: Callable[..., Any],
        signature: str | None = None,
    ) -> None:
        """Makes `implementation` available as `$name` in every evaluation. With a `signature`
        (such as `<n-n?:n>`), arguments are checked before each call, as for built-in
        functions. A function of the same name in the bindings takes precedence. Raises a
        `JlensError` when the signature is invalid."""

@final
class Document:
    """A JSON document converted once, for evaluating several expressions (or one several
    times) against it. It can be shared between threads."""

    def __new__(cls, value: Any) -> Self:
        """Converts a Python value, with the rules of `Expression.evaluate`'s input."""

    @staticmethod
    def from_json(text: str, read: Read | None = None) -> Document:
        """Parses JSON text. Raises a `ValueError` when it is not valid JSON. With `read` (the
        `"input"` of `reads()` or of `union_reads()`), the fields it leaves out are skipped,
        checked as JSON syntax only, and the document serves the expressions it comes from as
        the whole one would."""

@final
class Function:
    """A JSONata function passed to a host function, such as the lambda in
    `$retry(function() {...}, 3)`. Calling it applies the function with the context value
    `null`; arguments and the result are converted as for a host function. It is valid until the
    host function returns: calling it later raises a `JlensError` with code `U1005`."""

    @property
    def arity(self) -> int:
        """The number of parameters the function declares."""

    def __call__(self, *args: Any) -> Any:
        """Applies the function. Raises a `JlensError`, or the exception a host function
        raised, when it fails."""

def union_reads(reads: list[Reads] | tuple[Reads, ...]) -> Reads:
    """What any of several expressions reads: an input cut down to the union gives each of them
    its result. Raises a `ValueError` when an item is not in the form `Expression.reads`
    returns."""

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
