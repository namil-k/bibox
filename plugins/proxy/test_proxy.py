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

ACM = {"bibtex_key": "matejka2017", "doi": "10.1145/3025453.3025912", "url": "https://dl.acm.org/doi/10.1145/3025453.3025912"}
URL_ONLY = {"bibtex_key": "web2020", "url": "https://example.org/paper"}
BARE = {"bibtex_key": "bare1999", "title": "Nothing to link"}

KHU = "https://openlink.khu.ac.kr/link.n2s?url="
EZ = "https://ezproxy.example.edu/login?url="
ATHENS = "https://go.openathens.net/redirector/example.edu?url={url_encoded}"
OPENURL = "https://resolver.example.edu/openurl?sid=bibox&id=doi:{doi}"


class LinkTests(unittest.TestCase):
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
        self.config["link"] = EZ
        r = main.open_cmd({"command": "open", "trigger": "key", "entry": ACM, "entries": [ACM, URL_ONLY, BARE]})
        self.assertEqual(r, "opened 2, skipped 1 (no DOI or URL)")
        self.assertEqual(sorted(self.opened(2)), sorted([EZ + "https://doi.org/10.1145/3025453.3025912", EZ + "https://example.org/paper"]), "each once; the two openers run concurrently so order is free")

    def test_open_cmd_uses_the_cursor_entry_when_nothing_is_selected(self):
        self.config["link"] = KHU
        r = main.open_cmd({"command": "open", "trigger": "menu", "entry": URL_ONLY, "entries": []})
        self.assertEqual(r, "opened 1")
        self.assertEqual(self.opened(1), [KHU + "https://example.org/paper"])

    def test_open_cmd_without_a_link_tells_where_to_set_it(self):
        r = main.open_cmd({"command": "open", "trigger": "key", "entry": ACM, "entries": [ACM]})
        self.assertTrue(r.startswith("Set proxy.link in Settings"), r)
        self.assertFalse(os.path.exists(self.log), "nothing opened")


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

    def test_url_reads_the_link_from_config_toml_when_no_flag_is_given(self):
        with open(os.path.join(self.tmp, "config.toml"), "w") as f:
            f.write('bibox_dir = "/tmp/x"\n\n[plugins.proxy]\nlink = "{}"\n'.format(OPENURL))
        p = self.run_cli("url", "matejka2017", config_dir=self.tmp)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout.strip(), "https://resolver.example.edu/openurl?sid=bibox&id=doi:10.1145/3025453.3025912")

    def test_url_fails_clearly_without_a_link_or_without_an_identifier(self):
        p = self.run_cli("url", "matejka2017")
        self.assertEqual(p.returncode, 1)
        self.assertIn("--link", p.stderr)
        p = self.run_cli("url", "bare1999", "--link", EZ)
        self.assertEqual(p.returncode, 1)
        self.assertIn("no DOI or URL", p.stderr)
        p = self.run_cli("url", "nosuch", "--link", EZ)
        self.assertEqual(p.returncode, 1)
        self.assertIn("Entry not found", p.stderr)


if __name__ == "__main__":
    unittest.main()
