"""Makes Alegreya Sans show ordinary lining, equal-width digits by default.

    python art/make_fonts.py

Alegreya Sans defaults to old-style figures, where "1" looks like a small "ı" ("FM1" reads "FMı"),
and egui can't switch OpenType features on. This bakes the font's own 'lnum' and 'tnum'
substitutions into its character map and renames the family to mark it as modified, which the
SIL Open Font License allows (no Reserved Font Name is declared). Safe to run more than once.
"""

from pathlib import Path

from fontTools.ttLib import TTFont

FONTS = Path(__file__).resolve().parent.parent / "assets" / "fonts"
FILES = ["AlegreyaSans-Regular.ttf", "AlegreyaSans-Medium.ttf", "AlegreyaSans-Bold.ttf"]
DIGITS = "0123456789"


def single_substitutions(font, tag):
    gsub = font["GSUB"].table
    mapping = {}
    for record in gsub.FeatureList.FeatureRecord:
        if record.FeatureTag != tag:
            continue
        for index in record.Feature.LookupListIndex:
            lookup = gsub.LookupList.Lookup[index]
            for sub in lookup.SubTable:
                if lookup.LookupType == 7:  # extension wrapper
                    sub = sub.ExtSubTable
                if hasattr(sub, "mapping"):
                    mapping.update(sub.mapping)
    return mapping


def main():
    for name in FILES:
        path = FONTS / name
        font = TTFont(path)
        lnum = single_substitutions(font, "lnum")
        tnum = single_substitutions(font, "tnum")
        changed = 0
        for table in font["cmap"].tables:
            for ch in DIGITS:
                glyph = table.cmap.get(ord(ch))
                if glyph is None:
                    continue
                lined = tnum.get(lnum.get(glyph, glyph), lnum.get(glyph, glyph))
                if lined != glyph:
                    table.cmap[ord(ch)] = lined
                    changed += 1
        for record in font["name"].names:
            if record.nameID in (1, 4, 16) and "Lining" not in record.toUnicode():
                record.string = record.toUnicode().replace("Alegreya Sans", "Alegreya Sans Lining")
        font.save(path)
        print(f"{name}: {changed} digit mappings switched to lining figures")


if __name__ == "__main__":
    main()
