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

DATA_VERSION = 2

QLEVER = "https://qlever.dev/api/wikidata"
CENSUS_SURNAMES = "https://www2.census.gov/topics/genealogy/2010surnames/names.zip"
GUTENBERG = "https://www.gutenberg.org/cache/epub/{id}/pg{id}.txt"
GEONAMES_POSTAL = "https://download.geonames.org/export/zip/{file}"
LIBADDRESSINPUT = "https://raw.githubusercontent.com/google/libaddressinput/master/testdata/countryinfo.txt"

# locale id -> (ISO country, GeoNames postal archive, English country name)
PLACES = {
    "en-US": ("US", "US.zip", "United States"),
    "en-GB": ("GB", "GB_full.csv.zip", "United Kingdom"),
    "en-IN": ("IN", "IN.zip", "India"),
    "de": ("DE", "DE.zip", "Germany"),
    "fr": ("FR", "FR.zip", "France"),
    "es": ("ES", "ES.zip", "Spain"),
    "pt-BR": ("BR", "BR.zip", "Brazil"),
    "ja": ("JP", "JP.zip", "Japan"),
}
PLACES_PER_LOCALE = 3000
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


def address_formats(cache):
    """libaddressinput country records keyed by ISO code."""
    records = {}
    for line in fetch(LIBADDRESSINPUT, cache).decode("utf-8").splitlines():
        match = re.match(r"data/([A-Z]{2})=(\{.*\})$", line.strip())
        if match:
            records[match.group(1)] = json.loads(match.group(2))
    return records


def sub_lookup(record, names_key):
    keys = record.get("sub_keys", "").split("~")
    names = record.get(names_key, "").split("~")
    return {fold_ascii(n).lower(): k for k, n in zip(keys, names) if n}


def geonames_postal(archive, country, cache):
    raw = fetch(GEONAMES_POSTAL.format(file=archive), cache)
    zipped = zipfile.ZipFile(io.BytesIO(raw))
    name = next(n for n in zipped.namelist() if n.endswith(".txt") and n.startswith(country))
    for line in zipped.read(name).decode("utf-8").splitlines():
        fields = [f.strip() for f in line.split("\t")]
        if len(fields) >= 9 and fields[0] == country:
            yield fields


def japanese_municipalities(cache):
    """Kanji names of Japanese municipalities keyed by 5-digit JIS code and
    by (prefecture code, romaji name)."""
    query = PREFIXES + """
SELECT ?code ?ja ?en WHERE {
  ?m wdt:P429 ?code ; rdfs:label ?ja . FILTER(LANG(?ja) = "ja")
  OPTIONAL { ?m rdfs:label ?en . FILTER(LANG(?en) = "en") }
  FILTER NOT EXISTS { ?m wdt:P576 ?end }
}"""
    by_code, by_name = {}, {}
    for row in sparql(query, cache):
        code = re.sub(r"\D", "", row["code"])[:5]
        if len(code) != 5 or not JAPANESE_PLACE.match(row["ja"]):
            continue
        by_code.setdefault(code, row["ja"])
        if row.get("en"):
            key = (code[:2], fold_ascii(row["en"]).lower().split(",")[0].strip())
            by_name.setdefault(key, row["ja"])
    return by_code, by_name


JAPANESE_PLACE = re.compile(r"^[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fffヶヵ々]{2,12}$")
JP_SUFFIXES = ("Shi", "Ku", "Cho", "Machi", "Mura", "Son", "Gun", "Ken", "To", "Fu")
# Postcodes assigned to a single organisation, not a place.
ORGANISATION = re.compile(
    r"GmbH|\b(AG|KG|SE|eG|e\.\s?V|Ltd|Inc|SAS?|Postfach)\b|\b[A-ZÄÖÜ]{2,}\b|"
    r"bank|kasse|amt\b|Universit|Klinik|Krankenhaus|Verlag|Zentrale|Verwaltung|Behörde|"
    r"Stadtwerke|Versicherung|Hauptniederlassung")


def is_place_name(locality):
    return len(locality.split()) <= 5 and not ORGANISATION.search(locality)


def romaji_stem(name):
    words = name.split()
    if len(words) > 1 and words[-1] in JP_SUFFIXES:
        words = words[:-1]
    return fold_ascii(" ".join(words)).lower()


def place_rows(locale, country, archive, formats, cache):
    """(postcode, locality, region, region code) tuples for one locale."""
    record = formats.get(country, {})
    rows = []
    if country == "JP":
        prefectures = record["sub_keys"].split("~")
        prefecture_codes = {
            lname.lower(): f"{i + 1:02d}" for i, lname in enumerate(record["sub_lnames"].split("~"))
        }
        prefecture_codes["gumma"] = prefecture_codes["gunma"]  # GeoNames' older spelling
        by_code, by_name = japanese_municipalities(cache)
        for f in geonames_postal(archive, country, cache):
            pref_code = prefecture_codes.get(romaji_stem(f[3]))
            if not pref_code:
                continue
            code = f[8] if len(f) > 8 and re.fullmatch(r"\d{5}", f[8] or "") else ""
            municipality = by_code.get(code) if code and not f[5].endswith("Shi") else None
            municipality = municipality or by_name.get((pref_code, romaji_stem(f[5])))
            if municipality:
                rows.append((f[1], municipality, prefectures[int(pref_code) - 1], ""))
        return rows
    uf = sub_lookup(record, "sub_names")
    for f in geonames_postal(archive, country, cache):
        postal, locality, admin1, admin1_code, admin2 = f[1], f[2], f[3], f[4], f[5]
        # German large-customer postcodes (agencies, firms) carry no accuracy.
        if country == "DE" and (len(f) < 12 or not f[11]):
            continue
        if (not postal or not locality or re.search(r"\d|cedex|[()]", locality, re.I)
                or not is_place_name(locality)):
            continue
        if country == "US":
            # Military mail (APO/FPO) codes are not places.
            if admin1_code in ("AA", "AE", "AP"):
                continue
            region, code = admin1, admin1_code
        elif country in ("GB", "ES"):
            region, code = admin2 or admin1, ""
        elif country == "BR":
            region, code = admin1, uf.get(fold_ascii(admin1).lower(), "")
        else:
            region, code = admin1, ""
        if region:
            rows.append((postal, locality, region, code))
    return rows


def valid_postcodes(rows, record):
    """Keep rows whose postcode matches the country's libaddressinput
    pattern (drops CEDEX and other special postcodes)."""
    pattern = re.compile(f"(?:{record.get('zip', '.*')})")
    return [row for row in rows if pattern.fullmatch(row[0])]


def sample(rows, limit):
    """A stable, source-order-independent sample of distinct rows."""
    unique = sorted(set(rows), key=lambda r: hashlib.sha256("\t".join(r).encode()).hexdigest())
    return unique[:limit]


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

    formats = address_formats(cache)
    place_summary = []
    country_rows = []
    for locale, (country, archive, english) in PLACES.items():
        rows = valid_postcodes(place_rows(locale, country, archive, formats, cache), formats[country])
        rows = sample(rows, PLACES_PER_LOCALE)
        write_tsv(out / "places" / f"{locale}.tsv", rows)
        record = formats[country]
        country_rows.append((locale, country, english, record["fmt"], record.get("zip", "")))
        place_summary.append(f"| {locale} | {len(rows)} |")
        print(f"{locale}: {len(rows)} places", file=sys.stderr)
    write_tsv(out / "places" / "countries.tsv", country_rows)

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

| Locale | Places |
|---|---|
{chr(10).join(place_summary)}

## Formats

- `names/<locale>.given.tsv`: name, ASCII form, weight, gender (`m`/`f`).
- `names/<locale>.family.tsv`: name, ASCII form, weight.
- `text/en.sentences.txt`: one sentence per line.
- `text/en.words.tsv`: word, weight.
- `places/<locale>.tsv`: postcode, locality, region, region code (US state
  and Brazilian UF abbreviations; empty elsewhere).
- `places/countries.tsv`: locale, ISO country, English name, libaddressinput
  address format, postcode pattern.

Weights are occurrence counts in the source. For `ja`, names are kanji and
the ASCII form is the romaji reading.

## Sources and licenses

- Given names, and family names for every locale except en-US: Wikidata,
  people by citizenship, ranked by how many carry the name. CC0 1.0.
- en-US family names: US Census Bureau, 2010 surnames. Public domain.
- Postcodes, localities and regions: GeoNames postal code data,
  https://www.geonames.org/, licensed under CC BY 4.0. Japanese municipality
  names in kanji are joined from Wikidata (CC0 1.0).
- Address formats, postcode patterns and subdivision codes: Google
  libaddressinput, Apache License 2.0.
- English text: sentences sampled from public-domain novels (Austen, Doyle,
  Shelley, Dickens, Melville, Stoker, Wilde, C. Brontë), taken from Project
  Gutenberg editions with the Project Gutenberg header and license removed.
"""
    (out / "MANIFEST.md").write_text(manifest, encoding="utf-8")


if __name__ == "__main__":
    main()
