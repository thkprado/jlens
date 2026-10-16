"""JSONata for Python: jlens, a JSONata engine written in Rust.

Compile an expression once with :class:`Expression`, then evaluate it against any number of
documents. A document evaluated many times can be converted once into a :class:`Document`.
Errors raise :class:`JlensError`, which carries the JSONata error code.
"""

from ._jlens import Document, Expression, Function, JlensError, __version__, union_reads
from ._types import AuditResult, CaptureLimits, ErrorInfo, Read, Reads

__all__ = [
    "AuditResult",
    "CaptureLimits",
    "Document",
    "ErrorInfo",
    "Expression",
    "Function",
    "JlensError",
    "Read",
    "Reads",
    "__version__",
    "union_reads",
]
