"""Verify escaped, offline build notices and the shared resolved inventory."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import open_source


class NoticeTests(unittest.TestCase):
    def test_metadata_cannot_install_markup_or_unsafe_links(self):
        package = {"name": '<script>alert(1)</script>', "version": '"1', "license": "MIT",
                   "url": 'javascript:alert(1)', "notices": [('LICENSE', '<img src=x onerror=alert(1)>')]}
        markup = open_source.package_html(package)
        self.assertNotIn('<script>', markup)
        self.assertNotIn('<img', markup)
        self.assertNotIn('href=', markup)
        self.assertIn('&lt;script&gt;', markup)
        self.assertIn('&lt;img', markup)

    def test_notice_discovery_includes_nested_attribution_but_not_source_or_external_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.toml').touch()
            (root / 'LICENSE-MIT').write_text('license')
            (root / 'vendor').mkdir()
            (root / 'vendor/NOTICE').write_text('attribution')
            (root / 'vendor/source.rs').write_text('source')
            (root / 'LICENSE-link').symlink_to(root / 'vendor/source.rs')
            package = {'manifest': str(root / 'Cargo.toml'), 'license_file': None}
            self.assertEqual({path.name for path in open_source.notice_files(package)}, {'LICENSE-MIT', 'NOTICE'})

    def test_build_override_is_exact_and_archive_fallback_is_explicit(self):
        with patch.dict(os.environ, {'OPENWEBIDE_BUILD_COMMIT': 'a' * 40}):
            self.assertEqual(open_source.build_commit(open_source.ROOT), 'a' * 40)
        with patch.dict(os.environ, {'OPENWEBIDE_BUILD_COMMIT': '<script>'}):
            with self.assertRaises(ValueError):
                open_source.build_commit(open_source.ROOT)
        with patch.dict(os.environ, {'OPENWEBIDE_BUILD_COMMIT': ''}), patch('open_source.subprocess.run') as run:
            run.return_value.returncode = 1
            self.assertEqual(open_source.build_commit(open_source.ROOT), 'unknown (source archive)')

    def test_resolved_inventory_retains_vendored_crates_and_bundled_asset_texts(self):
        sections, watched = open_source.render_app(open_source.ROOT)
        fragment = sections["about"] + sections["software"]
        self.assertIn('SIL OPEN FONT LICENSE', fragment)
        self.assertIn('Lucide icons', fragment)
        self.assertIn('Copyright (c) 2022 Greg Johnston', fragment)
        self.assertIn('tree-sitter-rust', fragment)
        self.assertIn('leptos', fragment)
        self.assertIn('Commit', fragment)
        self.assertIn('source archive', open_source.build_commit(Path(tempfile.gettempdir())))
        self.assertIn(open_source.ROOT / 'Cargo.lock', watched)
        page = open_source.render_page(sections, [Path('styles-123.css')])
        self.assertIn('href="/styles-123.css"', page)
        self.assertNotIn('/api/', page)
        self.assertNotIn('localStorage', page)

    def test_supplemental_sources_are_pinned_and_file_paths_cannot_escape(self):
        entries = json.loads((open_source.ROOT / 'third-party/notices/index.json').read_text())
        for notices in entries.values():
            for notice in notices:
                self.assertRegex(notice['source'], r'^https://raw\.githubusercontent\.com/[^/]+/[^/]+/([0-9a-f]{40}|0\.3\.9)/')
                self.assertTrue(open_source.supplemental_notices(open_source.ROOT, [notice]))
        with self.assertRaises(ValueError):
            open_source.supplemental_notices(open_source.ROOT, [{'file': '../secret', 'source': 'source'}])
