"""Offline unit tests: python3 -m unittest scripts/test_extract.py"""
import json, os, sys, tempfile, unittest
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract as x


class TestDetection(unittest.TestCase):
    def test_find_urls_strips_punctuation(self):
        t = "Código: https://github.com/midudev/aprendiendo-react. Y (https://bit.ly/abc)!"
        self.assertEqual(x.find_urls(t), ["https://github.com/midudev/aprendiendo-react", "https://bit.ly/abc"])

    def test_bare_github(self):
        self.assertIn("https://github.com/openai/whisper", x.find_urls("mira github.com/openai/whisper hoy"))

    def test_classify_github_with_path(self):
        r = x.classify_repo_url("https://github.com/user/proj/tree/main/examples/rag")
        self.assertEqual(r["repo"], "user/proj")
        self.assertEqual(r["path"], {"ref": "main", "path": "examples/rag"})

    def test_classify_rejects_non_repo(self):
        self.assertIsNone(x.classify_repo_url("https://github.com/features/copilot"))
        self.assertIsNone(x.classify_repo_url("https://github.com/sponsors/foo"))
        self.assertIsNone(x.classify_repo_url("https://www.youtube.com/watch?v=abc"))

    def test_classify_git_suffix_and_other_hosts(self):
        self.assertEqual(x.classify_repo_url("https://github.com/a/b.git")["repo"], "a/b")
        self.assertEqual(x.classify_repo_url("https://gitlab.com/g/sub/p/-/tree/x")["repo"], "g/sub/p")
        self.assertEqual(x.classify_repo_url("https://huggingface.co/spaces/o/n")["repo"], "spaces/o/n")

    def test_spoken_mentions(self):
        m = x.find_spoken_github("lo tenéis en github punto com barra midudev barra jscamp")
        self.assertEqual(m[0]["repo"], "midudev/jscamp")

    def test_platform_and_id(self):
        self.assertEqual(x.detect_platform("https://www.instagram.com/reel/Cxyz/"), "instagram")
        self.assertEqual(x.youtube_id("https://youtube.com/shorts/dQw4w9WgXcQ"), "dQw4w9WgXcQ")
        self.assertEqual(x.youtube_id("https://youtu.be/dQw4w9WgXcQ?t=3"), "dQw4w9WgXcQ")

    def test_shortener_and_code_host(self):
        self.assertTrue(x.is_shortener("https://bit.ly/x"))
        self.assertTrue(x.is_shortener("https://linktr.ee/someone"))
        self.assertTrue(x.is_code_host("https://colab.research.google.com/drive/1"))

    def test_signals(self):
        s = x.keyword_hits("En este vídeo te explico paso a paso, el repositorio está en GitHub")
        self.assertIn("paso a paso", s["explainer"])
        self.assertIn("github", s["repo"])

    def test_vtt(self):
        vtt = "WEBVTT\n\n00:00.000 --> 00:01.000\nhola <c>mundo</c>\n\n00:01.000 --> 00:02.000\nhola mundo\n"
        self.assertEqual(x._vtt_to_text(vtt), "hola mundo")


class TestInputs(unittest.TestCase):
    def _tmp(self, name, content):
        d = tempfile.mkdtemp()
        p = os.path.join(d, name)
        with open(p, "w", encoding="utf-8") as f:
            f.write(content)
        return p

    def test_instagram_saved_posts_json(self):
        data = {"saved_saved_media": [
            {"title": "dev", "string_map_data": {"Saved on": {"href": "https://www.instagram.com/reel/AAA/", "timestamp": 1}}},
            {"title": "x", "string_map_data": {"Saved on": {"href": "https://www.instagram.com/p/BBB/"}}},
        ]}
        self.assertEqual(x.load_inputs(self._tmp("saved_posts.json", json.dumps(data))),
                         ["https://www.instagram.com/reel/AAA/", "https://www.instagram.com/p/BBB/"])

    def test_takeout_csv(self):
        csv = "Video ID,Playlist Video Creation Timestamp\ndQw4w9WgXcQ,2024-01-01\n"
        self.assertEqual(x.load_inputs(self._tmp("Liked videos.csv", csv)),
                         ["https://www.youtube.com/watch?v=dQw4w9WgXcQ"])

    def test_txt(self):
        p = self._tmp("favs.txt", "https://youtu.be/dQw4w9WgXcQ\nhttps://www.instagram.com/reel/C1/\n")
        self.assertEqual(len(x.load_inputs(p)), 2)


if __name__ == "__main__":
    unittest.main()
