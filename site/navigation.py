"""Validate the curated navigation against the documentation snapshot."""

from collections import Counter


def validate_navigation(navigation, content):
    pages = []

    def visit(items):
        if not isinstance(items, list) or not items:
            raise ValueError("Navigation sections must be nonempty lists")
        for item in items:
            if not isinstance(item, dict) or len(item) != 1:
                raise ValueError("Each navigation entry must have one title")
            title, target = next(iter(item.items()))
            if not isinstance(title, str) or not title.strip():
                raise ValueError("Navigation titles must be nonempty strings")
            if isinstance(target, list):
                visit(target)
            elif isinstance(target, str):
                pages.append(target)
            else:
                raise ValueError(f"Invalid navigation target for {title}")

    visit(navigation)
    expected = {path.relative_to(content).as_posix() for path in content.rglob("*.md")}
    counts = Counter(pages)
    missing = sorted(expected - counts.keys())
    unknown = sorted(counts.keys() - expected)
    duplicates = sorted(page for page, count in counts.items() if count > 1)
    if missing or unknown or duplicates:
        raise ValueError(
            f"Navigation coverage: unlisted={missing}, missing files={unknown}, duplicates={duplicates}"
        )
    return navigation
