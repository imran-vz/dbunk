#!/usr/bin/env python3
"""Build the compiled-in Table Seeding data (ADR-0014, "Seed data variety").

Downloads public sources, filters and ranks them, and writes compact TSV and
text files under backend/data/seed/. The output is committed; rerun this
script only to refresh or extend the data, then bump DATA_VERSION here, in
backend/src/seed/data.rs, and SEED_GENERATOR_VERSION in backend/src/seed.rs,
because a seed reproduces its rows only on the same data.

Standard library only. Usage:

    python3 -I tools/seed-data/build.py [--out backend/data/seed] [--cache DIR]
"""

import argparse
import collections
import csv
import hashlib
import io
import json
import pathlib
import re
import sys
import unicodedata
import urllib.parse
import urllib.request
import zipfile

DATA_VERSION = 1

QLEVER = "https://qlever.dev/api/wikidata"
CENSUS_SURNAMES = "https://www2.census.gov/topics/genealogy/2010surnames/names.zip"
GUTENBERG = "https://www.gutenberg.org/cache/epub/{id}/pg{id}.txt"
USER_AGENT = "dbunk-seed-data-build/1 (https://github.com/imran-vz/dbunk)"

MALE = "Q6581097"
FEMALE = "Q6581072"

# locale id -> (Wikidata country, label language, family source)
LOCALES = {
    "en-US": ("Q30", "en", "census"),
    "en-GB": ("Q145", "en", "wikidata"),
    "en-IN": ("Q668", "en", "wikidata"),
    "de": ("Q183", "de", "wikidata"),
    "fr": ("Q142", "fr", "wikidata"),
    "es": ("Q29", "es", "wikidata"),
    "pt-BR": ("Q155", "pt", "wikidata"),
    "ja": ("Q17", "ja", "wikidata"),
}

GIVEN_PER_GENDER = 500
FAMILY_NAMES = 1500
MIN_COUNT = 3

# Public-domain English prose. Sentences are sampled, never whole passages.
BOOKS = [1342, 1661, 84, 98, 2701, 345, 174, 1260]
SENTENCES = 5000
WORDS = 3000

LATIN_NAME = re.compile(r"^[A-Za-zÀ-ÖØ-öø-ÿĀ-žḀ-ỿ][A-Za-zÀ-ÖØ-öø-ÿĀ-žḀ-ỿ'\- ]{0,23}$")
JAPANESE_NAME = re.compile(r"^[぀-ヿ㐀-䶿一-鿿々ヶ]{1,4}$")
ROMAJI = re.compile(r"^[A-Za-zĀāĒēĪīŌōŪū'\- ]{1,24}$")

FOLD = {"ß": "ss", "æ": "ae", "Æ": "Ae", "ø": "o", "Ø": "O", "œ": "oe", "Œ": "Oe",
        "ł": "l", "Ł": "L", "đ": "d", "Đ": "D", "ð": "d", "þ": "th", "ı": "i", "’": "'"}


# German spells umlauts out when they can't be written (Müller -> Mueller).
GERMAN_FOLD = {"ä": "ae", "ö": "oe", "ü": "ue", "Ä": "Ae", "Ö": "Oe", "Ü": "Ue"}


def fold_ascii(value, locale=""):
    if locale == "de":
        value = "".join(GERMAN_FOLD.get(ch, ch) for ch in value)
    value = "".join(FOLD.get(ch, ch) for ch in value)
    value = unicodedata.normalize("NFKD", value)
    value = "".join(ch for ch in value if not unicodedata.combining(ch))
    return value if value.isascii() else ""


def fetch(url, cache, data=None, accept=None):
    key = hashlib.sha256((url + (data or "")).encode()).hexdigest()[:24]
    path = cache / key
    if path.exists():
        return path.read_bytes()
    headers = {"User-Agent": USER_AGENT}
    if accept:
        headers["Accept"] = accept
    body = data.encode() if data is not None else None
    request = urllib.request.Request(url, data=body, headers=headers)
    with urllib.request.urlopen(request, timeout=300) as response:
        payload = response.read()
    path.write_bytes(payload)
    return payload


def sparql(query, cache):
    data = urllib.parse.urlencode({"query": query})
    raw = fetch(QLEVER, cache, data=data, accept="application/sparql-results+json")
    rows = json.loads(raw)["results"]["bindings"]
    return [{k: v["value"] for k, v in row.items()} for row in rows]


PREFIXES = """PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
"""


def wikidata_names(country, lang, prop, gender, limit, cache):
    gender_clause = f"wdt:P21 wd:{gender} ;" if gender else ""
    query = PREFIXES + f"""
SELECT ?label ?en (COUNT(?p) AS ?n) WHERE {{
  ?p wdt:P31 wd:Q5 ; wdt:P27 wd:{country} ; {gender_clause} wdt:{prop} ?name .
  ?name rdfs:label ?label . FILTER(LANG(?label) = "{lang}")
  OPTIONAL {{ ?name rdfs:label ?en . FILTER(LANG(?en) = "en") }}
}} GROUP BY ?label ?en ORDER BY DESC(?n) LIMIT {limit * 3}"""
    return [(r["label"], r.get("en", ""), int(r["n"])) for r in sparql(query, cache)]


def japanese_given(gender, limit, cache):
    # Wikidata's Japanese given-name items are kana readings. The kanji form
    # is recovered from each person's full Japanese label minus the kanji
    # family name, and the romaji from the given-name item's English label.
    query = PREFIXES + f"""
SELECT ?given ?romaji (COUNT(?p) AS ?n) WHERE {{
  ?p wdt:P31 wd:Q5 ; wdt:P27 wd:Q17 ; wdt:P21 wd:{gender} ;
     wdt:P734 ?fam ; wdt:P735 ?gn ; rdfs:label ?full .
  FILTER(LANG(?full) = "ja")
  ?fam rdfs:label ?famja . FILTER(LANG(?famja) = "ja")
  ?gn rdfs:label ?romaji . FILTER(LANG(?romaji) = "en")
  FILTER(STRSTARTS(STR(?full), STR(?famja)))
  BIND(REPLACE(SUBSTR(STR(?full), STRLEN(STR(?famja)) + 1), "^[ 　]+", "") AS ?given)
  FILTER(STRLEN(?given) >= 1 && STRLEN(?given) <= 4)
}} GROUP BY ?given ?romaji ORDER BY DESC(?n) LIMIT {limit * 3}"""
    return [(r["given"], r["romaji"], int(r["n"])) for r in sparql(query, cache)]


def clean(rows, locale, limit):
    """Validate, fold to ASCII, merge duplicates and keep the top `limit`."""
    merged = collections.OrderedDict()
    for label, english, count in rows:
        if count < MIN_COUNT:
            continue
        label = label.strip()
        if locale == "ja":
            romaji = english.strip()
            if not JAPANESE_NAME.match(label) or not ROMAJI.match(romaji):
                continue
            ascii_form = fold_ascii(romaji)
        else:
            if not LATIN_NAME.match(label) or label.isupper() and len(label) > 3:
                continue
            ascii_form = fold_ascii(label, locale)
        if not ascii_form or not re.search(r"[A-Za-z]", ascii_form):
            continue
        key = (label, ascii_form)
        merged[key] = merged.get(key, 0) + count
    ranked = sorted(merged.items(), key=lambda item: (-item[1], item[0]))
    return [(label, ascii_form, count) for (label, ascii_form), count in ranked[:limit]]


def census_surnames(cache, limit):
    archive = zipfile.ZipFile(io.BytesIO(fetch(CENSUS_SURNAMES, cache)))
    text = archive.read("Names_2010Census.csv").decode("latin-1")
    rows = []
    for record in csv.DictReader(io.StringIO(text)):
        name = record["name"]
        if name == "ALL OTHER NAMES" or not name.isalpha():
            continue
        rows.append((surname_case(name), "", int(record["count"])))
        if len(rows) >= limit:
            break
    return rows


def surname_case(upper):
    name = upper.capitalize()
    for prefix in ("Mc", "Mac", "O'"):
        if name.startswith(prefix) and len(name) > len(prefix) + 2:
            if prefix == "Mac" and name[3] in "hkek":
                break
            return prefix + name[len(prefix)].upper() + name[len(prefix) + 1:]
    return name


STOPWORDS = set("""a about above after again against all am an and any are as at be because been
before being below between both but by can did do does doing down during each few for from further
had has have having he her here hers herself him himself his how i if in into is it its itself just
me more most my myself no nor not now of off on once only or other our ours ourselves out over own
same she should so some such than that the their theirs them themselves then there these they this
those through to too under until up very was we were what when where which while who whom why will
with would you your yours yourself yourselves upon shall could might must said mr mrs miss sir one
two three may yet thus though whose every""".split())

SENTENCE_END = re.compile(r"(?<=[.!?])\s+(?=[A-Z])")
SENTENCE_OK = re.compile(r"^[A-Z][A-Za-z ,;:'\-]+[.!?]$")


def gutenberg_body(raw):
    text = raw.decode("utf-8", errors="replace").replace("\r\n", "\n")
    start = re.search(r"\*\*\* ?START OF (THE|THIS) PROJECT GUTENBERG[^\n]*\n", text)
    end = re.search(r"\*\*\* ?END OF (THE|THIS) PROJECT GUTENBERG", text)
    return text[start.end() if start else 0:end.start() if end else len(text)]


def english_text(cache):
    sentences = set()
    for book in BOOKS:
        body = gutenberg_body(fetch(GUTENBERG.format(id=book), cache))
        body = body.replace("’", "'").replace("‘", "'").replace("—", ", ").replace("--", ", ")
        for paragraph in re.split(r"\n\s*\n", body):
            flat = " ".join(paragraph.split())
            for sentence in SENTENCE_END.split(flat):
                sentence = sentence.strip()
                words = sentence.split()
                if (40 <= len(sentence) <= 240 and len(words) >= 6 and SENTENCE_OK.match(sentence)
                        and "  " not in sentence and ", ," not in sentence
                        and not any(w.isupper() and len(w) > 1 for w in words)):
                    sentences.add(sentence)
    ordered = sorted(sentences, key=lambda s: hashlib.sha256(s.encode()).hexdigest())
    chosen = ordered[:SENTENCES]
    counts = collections.Counter()
    for sentence in chosen:
        for word in re.findall(r"[a-z]+", sentence.lower()):
            if 3 <= len(word) <= 14 and word not in STOPWORDS:
                counts[word] += 1
    words = [(w, n) for w, n in sorted(counts.items(), key=lambda i: (-i[1], i[0])) if n >= 2][:WORDS]
    return chosen, words


def write_tsv(path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as out:
        for row in rows:
            fields = [str(f) for f in row]
            assert all("\t" not in f and "\n" not in f for f in fields), row
            out.write("\t".join(fields) + "\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default="backend/data/seed")
    parser.add_argument("--cache", default="/tmp/dbunk-seed-data-cache")
    args = parser.parse_args()
    out = pathlib.Path(args.out)
    cache = pathlib.Path(args.cache)
    cache.mkdir(parents=True, exist_ok=True)

    summary = []
    for locale, (country, lang, family_source) in LOCALES.items():
        given = []
        for gender, tag in ((MALE, "m"), (FEMALE, "f")):
            if locale == "ja":
                rows = japanese_given(gender, GIVEN_PER_GENDER, cache)
            else:
                rows = wikidata_names(country, lang, "P735", gender, GIVEN_PER_GENDER, cache)
            given += [(*row, tag) for row in clean(rows, locale, GIVEN_PER_GENDER)]
        if family_source == "census":
            family_rows = census_surnames(cache, FAMILY_NAMES * 2)
        else:
            family_rows = wikidata_names(country, lang, "P734", None, FAMILY_NAMES, cache)
        family = clean(family_rows, locale, FAMILY_NAMES)
        write_tsv(out / "names" / f"{locale}.given.tsv", given)
        write_tsv(out / "names" / f"{locale}.family.tsv", family)
        summary.append(f"| {locale} | {len(given)} | {len(family)} |")
        print(f"{locale}: {len(given)} given, {len(family)} family", file=sys.stderr)

    sentences, words = english_text(cache)
    (out / "text").mkdir(parents=True, exist_ok=True)
    (out / "text" / "en.sentences.txt").write_text("\n".join(sentences) + "\n", encoding="utf-8")
    write_tsv(out / "text" / "en.words.tsv", words)
    print(f"text: {len(sentences)} sentences, {len(words)} words", file=sys.stderr)

    manifest = f"""# Table Seeding data

Generated by `tools/seed-data/build.py`. Do not edit by hand. Data version
{DATA_VERSION}; it must match `DATA_VERSION` in `backend/src/seed/data.rs`, and
changing it requires bumping `SEED_GENERATOR_VERSION` in `backend/src/seed.rs`.

| Locale | Given names | Family names |
|---|---|---|
{chr(10).join(summary)}

English text: {len(sentences)} sentences and {len(words)} words.

## Formats

- `names/<locale>.given.tsv`: name, ASCII form, weight, gender (`m`/`f`).
- `names/<locale>.family.tsv`: name, ASCII form, weight.
- `text/en.sentences.txt`: one sentence per line.
- `text/en.words.tsv`: word, weight.

Weights are occurrence counts in the source. For `ja`, names are kanji and
the ASCII form is the romaji reading.

## Sources and licenses

- Given names, and family names for every locale except en-US: Wikidata,
  people by citizenship, ranked by how many carry the name. CC0 1.0.
- en-US family names: US Census Bureau, 2010 surnames. Public domain.
- English text: sentences sampled from public-domain novels (Austen, Doyle,
  Shelley, Dickens, Melville, Stoker, Wilde, C. Brontë), taken from Project
  Gutenberg editions with the Project Gutenberg header and license removed.
"""
    (out / "MANIFEST.md").write_text(manifest, encoding="utf-8")


if __name__ == "__main__":
    main()
