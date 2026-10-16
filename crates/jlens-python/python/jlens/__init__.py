"""JSONata for Python: jlens, a JSONata engine written in Rust.

Compile an expression once with :class:`Expression`, then evaluate it against any number of
documents. A document evaluated many times can be converted once into a :class:`Document`.
Errors raise :class:`JlensError`, which carries the JSONata error code.
"""

from ._jlens import Document, Expression, JlensError, __version__

__all__ = ["Document", "Expression", "JlensError", "__version__"]
