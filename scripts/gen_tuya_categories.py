"""Tuya's own category code list -> scripts/data/tuya_categories.json (input of build_rules.py).

Usage: python gen_tuya_categories.py [page.html]   (fetches the page when no file is given)
Reads the "List of category code" table from Tuya's standard instruction set page. The JSON is generated;
hand edits are forbidden -- the decisions taken from it live in _tuya_standard_rules.json.
"""
import datetime
import html
import json
import pathlib
import re
import sys
import urllib.request

URL = "https://developer.tuya.com/en/docs/iot/standarddescription?id=K9i5ql6waswzq"
OUT = pathlib.Path(__file__).parent / "data/tuya_categories.json"


def rows(page: str) -> dict[str, str]:
    anchor = page.find("Ceiling fan light")          # a name only the category table has
    start, end = page.rfind("<table", 0, anchor), page.find("</table>", anchor)
    out: dict[str, str] = {}
    for tr in re.findall(r"<tr[^>]*>(.*?)</tr>", page[start:end], re.DOTALL):
        cells = [html.unescape(re.sub(r"<[^>]+>", "", c)).strip() for c in re.findall(r"<t[dh][^>]*>(.*?)</t[dh]>", tr, re.DOTALL)]
        if len(cells) == 2 and cells[0] != "Category code":
            assert cells[0] not in out, cells
            out[cells[0]] = cells[1]
    return out


if __name__ == "__main__":
    if len(sys.argv) > 1:
        page = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
    else:
        req = urllib.request.Request(URL, headers={"User-Agent": "Mozilla/5.0"})
        page = urllib.request.urlopen(req).read().decode("utf-8")
    categories = rows(page)
    assert len(categories) > 50, "the table was not found"
    doc = {"source": URL + "#title-6-List%20of%20category%20code",
           "retrieved": datetime.datetime.now(datetime.UTC).date().isoformat(), "categories": categories}
    OUT.write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(len(categories), "categories ->", OUT)
