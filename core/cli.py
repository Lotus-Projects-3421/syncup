"""Command-line argument helpers.

AppImages (and some desktop launchers) may forward literal field codes such as
``%U`` or ``%F`` as command-line arguments.  Those tokens are placeholders, not
paths, so they must never reach the application as a folder to open.
"""

from __future__ import annotations

# Field codes used by .desktop files.  Compare case-insensitively.
_PLACEHOLDER_ARGS = frozenset({"%u", "%f"})


def is_placeholder_arg(arg: str) -> bool:
    """Return True when *arg* is a desktop-entry field code, not a real path."""
    return str(arg).strip().lower() in _PLACEHOLDER_ARGS


def sanitize_argv(argv: list[str] | tuple[str, ...] | None) -> list[str]:
    """Return *argv* without standalone ``%U``/``%F`` placeholder tokens.

    The executable name is always preserved.  Real arguments (including paths
    that happen to contain a percent sign) are left untouched.
    """
    if not argv:
        return []

    cleaned = [str(argv[0])]
    for arg in argv[1:]:
        if is_placeholder_arg(arg):
            continue
        cleaned.append(str(arg))
    return cleaned


__all__ = ["is_placeholder_arg", "sanitize_argv"]
