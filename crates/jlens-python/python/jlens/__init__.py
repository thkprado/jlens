"""JSONata for Python: jlens, a JSONata engine written in Rust.

Compile an expression once with :class:`Expression`, then evaluate it against any number of
documents. A document evaluated many times can be converted once into a :class:`Document`.
Errors raise :class:`JlensError`, which carries the JSONata error code.
"""

from ._jlens import Document, Expression, Function, JlensError, __version__
from ._types import AuditResult, CaptureLimits, ErrorInfo

__all__ = [
    "AuditResult",
    "CaptureLimits",
    "Document",
    "ErrorInfo",
    "Expression",
    "Function",
    "JlensError",
    "__version__",
]
