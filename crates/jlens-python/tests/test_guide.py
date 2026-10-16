"""The code of the Python guide (docs/src/guides/python.md), which includes the scripts in
`guide/`, whole or between their `ANCHOR` comments. Running them here means every snippet of the
guide works."""

import runpy
from pathlib import Path

import pytest

SCRIPTS = sorted((Path(__file__).parent / "guide").glob("*.py"))


@pytest.mark.parametrize("script", SCRIPTS, ids=[script.stem for script in SCRIPTS])
def test_guide(script):
    runpy.run_path(str(script), run_name="__guide__")
