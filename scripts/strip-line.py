"""Build a story whose `ja` table is missing one line, for the localisation checks.

The `en` text stays, so a fallback has somewhere to go and the strict and lenient
policies can be told apart by their output. Used by CI and runnable by hand:

    python3 scripts/strip-line.py stories/localised.ron /tmp/partial.ron
"""

import re
import sys


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: strip-line.py <source.ron> <target.ron>", file=sys.stderr)
        return 2

    source, target = argv[1], argv[2]
    text = open(source, encoding="utf-8").read()
    marker = '"ja": {'
    if marker not in text:
        print(f"{source}: no `ja` table to strip", file=sys.stderr)
        return 1

    start = text.index(marker)
    before, after = text[:start], text[start:]
    stripped = re.sub(r'\s*"stay\.line": "[^"]*",', "", after, count=1)
    if stripped == after:
        # A check that silently stops checking is worse than no check: the fallback test
        # would pass on a story that is still complete.
        print(f"{source}: `ja` has no `stay.line` to remove", file=sys.stderr)
        return 1

    open(target, "w", encoding="utf-8").write(before + stripped)
    print(f"wrote {target} without ja/stay.line")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
