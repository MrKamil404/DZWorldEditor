"""Makes the pages of the GitHub wiki from the guide in docs/.

    python tools/make_wiki.py <folder of the wiki repository>

The guide (docs/guide.md) becomes the Home page, every other page of docs/ a page of its own,
with a sidebar listing them. Links between the pages and to the screenshots are rewritten for
the wiki; the screenshots are shown from the main repository, so they are not copied. Commit
and push the wiki folder afterwards.
"""
import os
import re
import sys

REPOSITORY = "MrKamil404/DZWorldEditor"
DOCS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs")

# The pages in the order of the sidebar: file in docs/, page of the wiki.
PAGES = [
    ("guide.md", "Home"),
    ("objects.md", "Objects"),
    ("terrain.md", "Terrain"),
    ("surfaces.md", "Surfaces"),
    ("roads.md", "Roads"),
    ("shapes.md", "Shapes-and-key-points"),
    ("satellite.md", "Satellite-image"),
    ("generators.md", "Generators-and-the-Content-Browser"),
    ("real-places.md", "Maps-of-real-places"),
    ("mission-editor.md", "Mission-editor"),
    ("team-work.md", "Team-work"),
]


def convert(text, names):
    # Links to other pages, with or without a part of the page.
    def link(m):
        target, anchor = m.group(2), m.group(3) or ""
        if target in names:
            return f"]({names[target]}{anchor})"
        return m.group(0)

    text = re.sub(r"\]\((\.?/?)([a-z0-9-]+\.md)(#[^)]*)?\)", link, text)
    # The README stays in the main repository.
    text = text.replace("](../README.md)", f"](https://github.com/{REPOSITORY}#readme)")
    # Screenshots, from the main repository.
    raw = f"https://raw.githubusercontent.com/{REPOSITORY}/main/docs/screenshots/"
    text = re.sub(r"\]\(screenshots/", "](" + raw, text)
    # The wiki shows the name of the page as its title, so the first heading would be twice.
    text = re.sub(r"\A# [^
]*
+", "", text)
    # The way back to the guide is the sidebar.
    text = re.sub(r"\n\[Back to the guide\]\(Home\)\n", "\n", text)
    return text


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = sys.argv[1]
    names = {f: page for f, page in PAGES}
    for f, page in PAGES:
        with open(os.path.join(DOCS, f), encoding="utf-8") as src:
            text = convert(src.read(), names)
        with open(os.path.join(out, page + ".md"), "w", encoding="utf-8", newline="\n") as dst:
            dst.write(text)
    sidebar = ["**DZ World Editor**", ""]
    for f, page in PAGES:
        title = "Guide" if page == "Home" else page.replace("-", " ")
        sidebar.append(f"* [{title}]({page})")
    sidebar += ["", f"[Download](https://github.com/{REPOSITORY}/releases/latest)"]
    with open(os.path.join(out, "_Sidebar.md"), "w", encoding="utf-8", newline="\n") as dst:
        dst.write("\n".join(sidebar) + "\n")
    print(f"{len(PAGES)} pages written to {out}")


if __name__ == "__main__":
    main()
