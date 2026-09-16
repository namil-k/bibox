"""Unit tests for the proxy plugin. Run: python3 -m unittest test_proxy -v"""
import os
import stat
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import main  # noqa: E402
import registry  # noqa: E402

ACM = {"bibtex_key": "matejka2017", "doi": "10.1145/3025453.3025912", "url": "https://dl.acm.org/doi/10.1145/3025453.3025912"}
URL_ONLY = {"bibtex_key": "web2020", "url": "https://example.org/paper"}
BARE = {"bibtex_key": "bare1999", "title": "Nothing to link"}

KHU = "https://openlink.khu.ac.kr/link.n2s?url="
EZ = "https://ezproxy.example.edu/login?url="
ATHENS = "https://go.openathens.net/redirector/example.edu?url={url_encoded}"
OPENURL = "https://resolver.example.edu/openurl?sid=bibox&id=doi:{doi}"


class LinkTests(unittest.TestCase):
    def test_parse_links_splits_on_whitespace_and_keeps_order(self):
        self.assertEqual(main.parse_links("\n" + EZ + "\n  " + KHU + "\n\n"), [EZ, KHU])
        self.assertEqual(main.parse_links(EZ + " " + ATHENS), [EZ, ATHENS], "one line with spaces works too")
        self.assertEqual(main.parse_links(""), [])
        self.assertEqual(main.parse_links(None), [])

    def test_label_and_host_strip_what_the_user_does_not_need_to_read(self):
        self.assertEqual(main.label(EZ), "ezproxy.example.edu/login?url=")
        self.assertEqual(main.host(EZ), "ezproxy.example.edu")
        self.assertEqual(main.host(ATHENS), "go.openathens.net")

    def test_target_url_prefers_doi_then_url_then_none(self):
        self.assertEqual(main.target_url(ACM), "https://doi.org/10.1145/3025453.3025912")
        self.assertEqual(main.target_url(URL_ONLY), "https://example.org/paper")
        self.assertIsNone(main.target_url(BARE))
        self.assertIsNone(main.target_url({"doi": " ", "url": ""}), "blank strings count as missing")

    def test_proxied_treats_a_bare_link_as_a_prefix(self):
        self.assertEqual(main.proxied(EZ, ACM), EZ + "https://doi.org/10.1145/3025453.3025912")
        self.assertEqual(main.proxied(KHU, URL_ONLY), KHU + "https://example.org/paper", "raw, no percent encoding")
        self.assertIsNone(main.proxied(EZ, BARE))

    def test_proxied_fills_url_encoded_and_doi_placeholders(self):
        self.assertEqual(main.proxied(ATHENS, ACM), "https://go.openathens.net/redirector/example.edu?url=https%3A%2F%2Fdoi.org%2F10.1145%2F3025453.3025912")
        self.assertEqual(main.proxied(OPENURL, ACM), "https://resolver.example.edu/openurl?sid=bibox&id=doi:10.1145/3025453.3025912")
        self.assertEqual(main.proxied("https://p.example/{url}", URL_ONLY), "https://p.example/https://example.org/paper")
        sici = {"doi": "10.1002/(SICI)1097-0258(19980430)17:8<857::AID-SIM777>3.0.CO;2-E"}
        self.assertEqual(main.proxied(OPENURL, sici), "https://resolver.example.edu/openurl?sid=bibox&id=doi:10.1002/%28SICI%291097-0258%2819980430%2917:8%3C857::AID-SIM777%3E3.0.CO;2-E", "only characters that break a query string are encoded")

    def test_proxied_refuses_an_empty_link_and_skips_an_entry_the_template_cannot_fill(self):
        with self.assertRaises(ValueError):
            main.proxied("", ACM)
        with self.assertRaises(ValueError):
            main.proxied("   ", ACM)
        self.assertIsNone(main.proxied(OPENURL, URL_ONLY), "a DOI template cannot use a URL-only entry")
        self.assertIsNone(main.proxied(ATHENS, BARE))


LIBPROXY = [
    {"name": "Seoul National University", "url": "https://ezproxy.snu.ac.kr/login?url=$@", "country": "South Korea", "location": {"lat": 1, "lng": 2}},
    {"name": "Yonsei", "url": "https://access.yonsei.ac.kr/link.n2s?url=$@", "country": "South Korea"},
    {"name": "Odd University", "url": "https://odd.example.edu/go?target=$@&campus=1", "country": "Nowhere"},
]

ZOTERO_RAW = """===== Asia =====
==== China ====
|Peking University|%%https://zm8lp2fe5j.search.serialssolutions.com/%%|
===== Africa =====
|University of Cape Town|%%https://uct.primo.exlibrisgroup.com/openurl/27UCT_INST/27UCT_INST:27UCT%%|
===== Europe =====
==== Germany ====
|Max Planck Society URL|%%http://sfx.mpg.de/sfx_local%%|
|Broken line without url|
"""


class RegistryTests(unittest.TestCase):
    def test_libproxy_entries_become_prefixes_or_url_templates(self):
        r = registry.convert_libproxy(LIBPROXY)
        self.assertEqual(r[0], {"name": "Seoul National University", "country": "South Korea", "link": "https://ezproxy.snu.ac.kr/login?url=", "kind": "proxy"})
        self.assertEqual(r[1]["link"], "https://access.yonsei.ac.kr/link.n2s?url=")
        self.assertEqual(r[2]["link"], "https://odd.example.edu/go?target={url}&campus=1", "$@ in the middle becomes {url}")

    def test_zotero_resolvers_get_a_doi_query_and_their_country(self):
        r = registry.parse_zotero(ZOTERO_RAW)
        self.assertEqual([e["name"] for e in r], ["Peking University", "University of Cape Town", "Max Planck Society URL"])
        self.assertEqual(r[0]["country"], "China")
        self.assertEqual(r[1]["country"], "Africa", "a region without country headings is the best we have")
        self.assertEqual(r[0]["link"], "https://zm8lp2fe5j.search.serialssolutions.com/?url_ver=Z39.88-2004&rfr_id=info:sid/bibox&rft_id=info:doi/{doi}")
        self.assertEqual(r[2]["kind"], "resolver")
        with_query = registry.parse_zotero("|X|%%https://x.example/openurl?vid=1%%|\n")
        self.assertTrue(with_query[0]["link"].startswith("https://x.example/openurl?vid=1&url_ver="), with_query[0]["link"])

    def test_search_matches_every_word_against_name_and_country_case_insensitively(self):
        entries = registry.convert_libproxy(LIBPROXY) + registry.parse_zotero(ZOTERO_RAW)
        self.assertEqual([e["name"] for e in registry.search(entries, "seoul")], ["Seoul National University"])
        self.assertEqual([e["name"] for e in registry.search(entries, "south korea")], ["Seoul National University", "Yonsei"])
        self.assertEqual([e["name"] for e in registry.search(entries, "university korea")], ["Seoul National University"])
        self.assertEqual(registry.search(entries, "nothing here"), [])
        self.assertEqual(registry.search(entries, "  "), [])

    def test_label_shows_name_country_and_where_the_link_goes(self):
        e = registry.convert_libproxy(LIBPROXY)[0]
        self.assertEqual(registry.label(e), "Seoul National University (South Korea)  ezproxy.snu.ac.kr")
        z = registry.parse_zotero(ZOTERO_RAW)[2]
        self.assertEqual(registry.label(z), "Max Planck Society URL (Germany)  resolver sfx.mpg.de")

    def test_load_reads_the_bundled_snapshot_and_it_is_not_tiny(self):
        entries = registry.load()
        self.assertGreater(len(entries), 1500)
        self.assertTrue(any(e["name"] == "Seoul National University" for e in entries))
        self.assertTrue(all(set(e) == {"name", "country", "link", "kind"} for e in entries))


OPENER = '''#!/bin/sh
echo "$1" >> "{log}"
'''


class OpenTests(unittest.TestCase):
    """open_cmd hands each proxied URL to $BIBOX_PROXY_OPENER (a recording script here) and reports counts."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="bibox-proxy-open-")
        self.log = os.path.join(self.tmp, "opened.txt")
        opener = os.path.join(self.tmp, "opener.sh")
        with open(opener, "w") as f:
            f.write(OPENER.format(log=self.log))
        os.chmod(opener, os.stat(opener).st_mode | stat.S_IEXEC)
        os.environ["BIBOX_PROXY_OPENER"] = opener
        os.environ["BIBOX_PLUGIN_DIR"] = HERE
        import bibox_plugin
        self.config = bibox_plugin.config
        self.config.clear()

    def tearDown(self):
        os.environ.pop("BIBOX_PROXY_OPENER", None)
        os.environ.pop("BIBOX_PLUGIN_DIR", None)
        self.config.clear()

    def opened(self, expect):
        for _ in range(40):
            if os.path.exists(self.log):
                with open(self.log) as f:
                    lines = [ln.strip() for ln in f if ln.strip()]
                if len(lines) >= expect:
                    return lines
            time.sleep(0.05)
        return []

    def test_open_cmd_launches_the_opener_once_per_entry_and_counts_skips(self):
        self.config["links"] = EZ
        r = main.open_cmd({"command": "open", "trigger": "key", "entry": ACM, "entries": [ACM, URL_ONLY, BARE]})
        self.assertEqual(r, "opened 2 via ezproxy.example.edu, skipped 1 (no DOI or URL)")
        self.assertEqual(sorted(self.opened(2)), sorted([EZ + "https://doi.org/10.1145/3025453.3025912", EZ + "https://example.org/paper"]), "each once; the two openers run concurrently so order is free")

    def test_open_cmd_uses_the_cursor_entry_when_nothing_is_selected(self):
        self.config["links"] = KHU
        r = main.open_cmd({"command": "open", "trigger": "menu", "entry": URL_ONLY, "entries": []})
        self.assertEqual(r, "opened 1 via openlink.khu.ac.kr")
        self.assertEqual(self.opened(1), [KHU + "https://example.org/paper"])

    def test_open_cmd_uses_the_first_link_when_several_are_set(self):
        self.config["links"] = EZ + "\n" + KHU
        r = main.open_cmd({"command": "open", "trigger": "key", "entry": ACM, "entries": [ACM]})
        self.assertEqual(r, "opened 1 via ezproxy.example.edu")
        self.assertEqual(self.opened(1), [EZ + "https://doi.org/10.1145/3025453.3025912"])

    def test_open_cmd_without_a_link_tells_where_to_set_it(self):
        r = main.open_cmd({"command": "open", "trigger": "key", "entry": ACM, "entries": [ACM]})
        self.assertTrue(r.startswith("Set proxy.links in Settings"), r)
        self.assertFalse(os.path.exists(self.log), "nothing opened")

    def pick_returning(self, answer):
        import bibox_plugin
        asked = []

        def fake_pick(title, items):
            asked.append((title, list(items)))
            return answer
        bibox_plugin.window.pick = staticmethod(fake_pick)
        return asked

    def test_choose_cmd_opens_through_the_picked_link(self):
        self.config["links"] = EZ + "\n" + KHU
        asked = self.pick_returning(1)
        r = main.choose_cmd({"command": "choose", "trigger": "key", "entry": ACM, "entries": [ACM]})
        self.assertEqual(asked, [("Open through", ["ezproxy.example.edu/login?url=", "openlink.khu.ac.kr/link.n2s?url="])])
        self.assertEqual(r, "opened 1 via openlink.khu.ac.kr")
        self.assertEqual(self.opened(1), [KHU + "https://doi.org/10.1145/3025453.3025912"])

    def test_find_cmd_puts_the_picked_library_first_and_saves_it(self):
        import bibox_plugin
        self.config["links"] = KHU
        saved = []
        bibox_plugin.window.prompt = staticmethod(lambda title, default="": "seoul national")
        asked = self.pick_returning(0)
        bibox_plugin.settings.set = staticmethod(lambda k, v: saved.append((k, v)))
        r = main.find_cmd({"command": "find", "trigger": "key"})
        self.assertEqual(asked[0][0], "Use")
        self.assertTrue(asked[0][1][0].startswith("Seoul National University (South Korea)"), asked[0][1])
        snu = next(e for e in registry.load() if e["name"] == "Seoul National University")
        self.assertEqual(saved, [("links", snu["link"] + "\n" + KHU)], "picked link first, the old default second")
        self.assertEqual(r, "Seoul National University is now your default library ({})".format(registry.host(snu["link"])))

    def test_find_cmd_reports_no_match_and_cancel(self):
        import bibox_plugin
        bibox_plugin.window.prompt = staticmethod(lambda title, default="": "zzzz no such place")
        r = main.find_cmd({"command": "find", "trigger": "key"})
        self.assertIn("no library matches", r)
        bibox_plugin.window.prompt = staticmethod(lambda title, default="": None)
        self.assertEqual(main.find_cmd({"command": "find", "trigger": "key"}), "cancelled")

    def test_choose_cmd_with_one_link_opens_without_asking_and_cancel_opens_nothing(self):
        self.config["links"] = EZ
        asked = self.pick_returning(0)
        r = main.choose_cmd({"command": "choose", "trigger": "key", "entry": ACM, "entries": [ACM]})
        self.assertEqual(r, "opened 1 via ezproxy.example.edu")
        self.assertEqual(asked, [], "one link: nothing to choose")
        self.config["links"] = EZ + "\n" + KHU
        self.pick_returning(None)
        r = main.choose_cmd({"command": "choose", "trigger": "key", "entry": URL_ONLY, "entries": [URL_ONLY]})
        self.assertEqual(r, "cancelled")
        self.assertEqual(self.opened(1), [EZ + "https://doi.org/10.1145/3025453.3025912"], "still only the first open")


FAKE_BIBOX = r'''#!/usr/bin/env python3
"""Answers `show <key> --json` with a canned entry."""
import json, sys
entries = {"matejka2017": {"bibtex_key": "matejka2017", "doi": "10.1145/3025453.3025912"}, "bare1999": {"bibtex_key": "bare1999"}}
if sys.argv[1] == "show" and sys.argv[2] in entries:
    print(json.dumps(entries[sys.argv[2]]))
else:
    print("Entry not found", file=sys.stderr)
    sys.exit(1)
'''


class CliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="bibox-proxy-cli-")
        self.bin = os.path.join(self.tmp, "fakebibox")
        with open(self.bin, "w") as f:
            f.write(FAKE_BIBOX)
        os.chmod(self.bin, os.stat(self.bin).st_mode | stat.S_IEXEC)

    def run_cli(self, *args, config_dir=None):
        env = dict(os.environ, BIBOX_CLI="1", BIBOX_BIN=self.bin)
        env.pop("BIBOX_CONFIG_DIR", None)
        if config_dir:
            env["BIBOX_CONFIG_DIR"] = config_dir
        return subprocess.run([sys.executable, os.path.join(HERE, "main.py")] + list(args), env=env, capture_output=True, text=True)

    def test_url_prints_the_proxied_url(self):
        p = self.run_cli("url", "matejka2017", "--link", EZ)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.strip(), EZ + "https://doi.org/10.1145/3025453.3025912")

    def write_config(self, *links):
        with open(os.path.join(self.tmp, "config.toml"), "w") as f:
            f.write('bibox_dir = "/tmp/x"\n\n[plugins.proxy]\nlinks = """\n{}\n"""\n'.format("\n".join(links)))

    def test_url_reads_the_first_link_from_config_toml_when_no_flag_is_given(self):
        self.write_config(OPENURL, EZ)
        p = self.run_cli("url", "matejka2017", config_dir=self.tmp)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.strip(), "https://resolver.example.edu/openurl?sid=bibox&id=doi:10.1145/3025453.3025912")

    def test_via_picks_a_link_by_number_or_by_a_piece_of_its_host(self):
        self.write_config(EZ, KHU)
        p = self.run_cli("url", "matejka2017", "--via", "khu", config_dir=self.tmp)
        self.assertEqual(p.stdout.strip(), KHU + "https://doi.org/10.1145/3025453.3025912", p.stderr)
        p = self.run_cli("url", "matejka2017", "--via", "2", config_dir=self.tmp)
        self.assertEqual(p.stdout.strip(), KHU + "https://doi.org/10.1145/3025453.3025912", p.stderr)
        p = self.run_cli("url", "matejka2017", "--via", "nowhere", config_dir=self.tmp)
        self.assertEqual(p.returncode, 1)
        self.assertIn("no link matches", p.stderr)

    def test_find_prints_matches_numbered_with_their_links(self):
        p = self.run_cli("find", "seoul national")
        self.assertEqual(p.returncode, 0, p.stderr)
        snu = next(e for e in registry.load() if e["name"] == "Seoul National University")
        self.assertIn("1  Seoul National University (South Korea)  " + snu["link"], p.stdout)
        p = self.run_cli("find", "zzzz no such place")
        self.assertEqual(p.returncode, 1)
        self.assertIn("no library matches", p.stderr)

    def test_list_prints_the_links_in_priority_order(self):
        self.write_config(EZ, KHU)
        p = self.run_cli("list", config_dir=self.tmp)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.splitlines(), ["1  " + EZ, "2  " + KHU])

    def test_url_fails_clearly_without_a_link_or_without_an_identifier(self):
        p = self.run_cli("url", "matejka2017")
        self.assertEqual(p.returncode, 1)
        self.assertIn("--link", p.stderr)
        self.assertIn("links", p.stderr)
        p = self.run_cli("url", "bare1999", "--link", EZ)
        self.assertEqual(p.returncode, 1)
        self.assertIn("no DOI or URL", p.stderr)
        p = self.run_cli("url", "nosuch", "--link", EZ)
        self.assertEqual(p.returncode, 1)
        self.assertIn("Entry not found", p.stderr)


if __name__ == "__main__":
    unittest.main()
