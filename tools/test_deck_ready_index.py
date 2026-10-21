"""Offline tests for deck_ready_index.py. Run: python -I tools/test_deck_ready_index.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import deck_ready_index as d  # noqa: E402


def proj(pid, name="N", **extra):
    return {"id": pid, "name": name, "repo": pid.split(":", 1)[-1], "first_seen": "2026-01-01", **extra}


class Rating(unittest.TestCase):
    def rating(self, assets):
        return d.rate(*d.classify(assets))

    def test_deck_beats_linux(self):
        self.assertEqual(self.rating(["game-steamdeck.tar.gz", "game-linux.zip"]), "deck_build")

    def test_linux_build(self):
        self.assertEqual(self.rating(["game-Linux-x86_64.AppImage", "game-win64.zip"]), "linux_build")

    def test_arm_only_linux_does_not_count(self):
        self.assertEqual(self.rating(["game-linux-aarch64.tar.gz", "game-win64.zip"]), "proton_only")

    def test_junk_ignored(self):
        self.assertEqual(self.rating(["game-linux.zip.sha256", "notes.txt"]), "source_only")

    def test_other_assets(self):
        self.assertEqual(self.rating(["data.bin"]), "other_assets")


class Release(unittest.TestCase):
    def rel(self, tag, assets, pre=False):
        return {"tagName": tag, "publishedAt": "x", "isDraft": False, "isPrerelease": pre,
                "releaseAssets": {"nodes": [{"name": a} for a in assets]}}

    def test_latest_stable_wins_over_newer_prereleases(self):
        window = [self.rel("nightly-2", ["g-win64.zip"], True), self.rel("nightly-1", ["g-win64.zip"], True)]
        latest = self.rel("v1", ["g-linux.zip"])
        self.assertEqual(d.pick_release(window, latest)["tagName"], "v1")

    def test_falls_back_to_window_without_latest(self):
        window = [self.rel("nightly-2", [], True), self.rel("v0", ["g-linux.zip"])]
        self.assertEqual(d.pick_release(window, None)["tagName"], "v0")

    def test_prerelease_only_project_uses_prerelease(self):
        self.assertEqual(d.pick_release([self.rel("b1", ["g-linux.zip"], True)], None)["tagName"], "b1")


class Misc(unittest.TestCase):
    def test_darwin_is_not_windows(self):
        _, _, _, win = d.classify(["vurus-darwin-arm64.zip", "vurus-win64.zip"])
        self.assertEqual(win, ["vurus-win64.zip"])

    def test_system_names_normalized(self):
        self.assertEqual({d.normalize_system(x) for x in ["Nintendo 64", "n64", "N64"]}, {"N64"})
        self.assertEqual({d.normalize_system(x) for x in ["PC Port", "PC (Windows)", "pc"]}, {"PC"})
        self.assertEqual(d.normalize_system("Odd Console"), "Odd Console")
        self.assertIsNone(d.normalize_system(None))

    def test_summary_caps_long_lists(self):
        added = [{"name": f"G{i}", "repo": f"a/g{i}", "id": f"github:a/g{i}"} for i in range(100)]
        text = d.render_summary({"added": added, "changed": [], "revived": [], "newly_missing": [],
                                 "still_missing": [], "removed": []}, 100, 100)
        self.assertIn("and 40 more", text)


class Slugs(unittest.TestCase):
    def test_github_slug_strips_git_suffix(self):
        self.assertEqual(d.github_slug("https://github.com/a/b.git"), "a/b")
        self.assertEqual(d.github_slug("https://github.com/a/b/releases/latest"), "a/b")

    def test_gitlab_slug(self):
        self.assertEqual(d.gitlab_slug("https://gitlab.com/g/sub/proj/-/releases"), "g/sub/proj")


class Merge(unittest.TestCase):
    def merge(self, new, old, today="2026-02-01"):
        return d.merge_with_previous(new, old, today)

    def test_first_run_adds_everything(self):
        out, ch = self.merge([proj("github:a/b")], [])
        self.assertEqual(out[0]["first_seen"], "2026-02-01")
        self.assertEqual(len(ch["added"]), 1)

    def test_unchanged_keeps_first_seen_and_reports_nothing(self):
        old = [proj("github:a/b")]
        out, ch = self.merge([proj("github:a/b")], old)
        self.assertEqual(out, old)
        self.assertFalse(any(ch[k] for k in ch))

    def test_changed_lists_fields(self):
        out, ch = self.merge([proj("github:a/b", deck_rating="deck_build")], [proj("github:a/b", deck_rating="linux_build")])
        self.assertEqual(ch["changed"][0][1], ["deck_rating"])

    def test_missing_is_dated_then_kept_then_removed(self):
        old = [proj("github:a/b")]
        out, ch = self.merge([], old, "2026-02-01")
        self.assertEqual(out[0]["missing_since"], "2026-02-01")
        self.assertEqual(len(ch["newly_missing"]), 1)
        out2, ch2 = self.merge([], out, "2026-02-04")
        self.assertEqual(len(out2), 1)
        self.assertEqual(len(ch2["still_missing"]), 1)
        out3, ch3 = self.merge([], out, "2026-02-08")
        self.assertEqual(out3, [])
        self.assertEqual(len(ch3["removed"]), 1)

    def test_revived_clears_missing_since(self):
        old = [proj("github:a/b", missing_since="2026-01-20")]
        out, ch = self.merge([proj("github:a/b")], old)
        self.assertNotIn("missing_since", out[0])
        self.assertEqual(len(ch["revived"]), 1)

    def test_renamed_repo_matches_old_entry_by_github_id(self):
        old = [proj("github:old/name", github_id=7, first_seen="2026-01-05")]
        out, ch = self.merge([proj("github:new/name", github_id=7)], old)
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]["first_seen"], "2026-01-05")
        self.assertEqual((len(ch["added"]), len(ch["newly_missing"])), (0, 0))
        self.assertEqual(ch["changed"][0][1], ["id", "repo"])

    def test_stranger_with_old_slug_does_not_inherit_history(self):
        old = [proj("github:old/name", github_id=7, first_seen="2026-01-05")]
        out, ch = self.merge([proj("github:new/name", github_id=7), proj("github:old/name", github_id=99)], old)
        stranger = next(p for p in out if p["github_id"] == 99)
        self.assertEqual(stranger["first_seen"], "2026-02-01")
        self.assertEqual([p["github_id"] for p in ch["added"]], [99])

    def test_output_sorted_by_name(self):
        out, _ = self.merge([proj("github:a/z", "Zed"), proj("github:a/a", "alpha")], [])
        self.assertEqual([p["name"] for p in out], ["alpha", "Zed"])


class Catalog(unittest.TestCase):
    def test_placeholder_source_is_not_a_source(self):
        import json, tempfile
        data = {"ports": [{"name": "A", "folder": "a", "source": "??", "tags": []},
                          {"name": "B", "folder": "b", "source": "https://github.com/x/y", "tags": []}]}
        with tempfile.TemporaryDirectory() as tmp:
            f = Path(tmp) / "ports.json"
            f.write_text(json.dumps(data), encoding="utf-8")
            cands = {}
            d.collect_catalog(cands, f)
        self.assertEqual(sorted(cands), ["catalog:a", "github:x/y"])


class Candidates(unittest.TestCase):
    def test_renamed_repo_merges_to_one_project(self):
        cands = {"github:old/name": {"key": "github:old/name", "host": "github", "repo": "old/name",
                                     "sources": ["catalog"], "name": "Game"},
                 "github:new/name": {"key": "github:new/name", "host": "github", "repo": "new/name",
                                     "sources": ["recomp_json"], "name": "Game"}}
        node = {"nameWithOwner": "new/name", "url": "https://github.com/new/name", "isArchived": False,
                "description": "A game", "repositoryTopics": {"nodes": []},
                "releases": {"nodes": [{"tagName": "v1", "publishedAt": "2026-01-01T00:00:00Z", "isDraft": False,
                                        "isPrerelease": False,
                                        "releaseAssets": {"nodes": [{"name": "game-linux-x64.zip"}]}}]},
                "recomp": None, "readme_mentions_deck": False}
        gh = {"old/name": {"status": "ok", "node": node}, "new/name": {"status": "ok", "node": node}}
        ready = d.ready_projects(cands, gh, {})
        self.assertEqual(len(ready), 1)
        self.assertEqual(ready[0]["id"], "github:new/name")
        self.assertEqual(ready[0]["sources"], ["catalog", "recomp_json"])

    def test_not_found_repo_is_not_ready(self):
        cands = {"github:a/b": {"key": "github:a/b", "host": "github", "repo": "a/b", "sources": ["catalog"],
                                "name": "G"}}
        self.assertEqual(d.ready_projects(cands, {"a/b": {"status": "not_found", "node": None}}, {}), [])

    def test_topic_only_without_game_signal_is_dropped(self):
        node = {"nameWithOwner": "a/lib", "url": "u", "isArchived": False, "description": "helper",
                "repositoryTopics": {"nodes": []},
                "releases": {"nodes": [{"tagName": "v1", "publishedAt": "x", "isDraft": False, "isPrerelease": False,
                                        "releaseAssets": {"nodes": [{"name": "lib-linux.zip"}]}}]},
                "recomp": None, "readme_mentions_deck": False}
        cands = {"github:a/lib": {"key": "github:a/lib", "host": "github", "repo": "a/lib", "sources": ["topic:decomp"]}}
        self.assertEqual(d.ready_projects(cands, {"a/lib": {"status": "ok", "node": node}}, {}), [])


if __name__ == "__main__":
    unittest.main()
