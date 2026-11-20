"""Generate CapCut-like drafts for recordings, clean or with mutated values.

Shapes follow what the Python core reads: subtitle texts linked to text segments,
TTS audio materials with speed/fade extras, video tracks, and nested combination
drafts under materials.drafts[].draft (AutoVideo style).
"""

from __future__ import annotations

import copy
import json
import random

TEXTS = ["Xin chào", "Hello world", "", "   ", "中文字幕", "😀 emoji", "line1\nline2",
         " nbsp", "Tiếng Việt có dấu", "a" * 40]
PH = "##_draftpath_placeholder_0A1B2C3D-AAAA-BBBB-1234-56789ABCDEF0_##"

# Values a mutation may put in place of a scalar.
WEIRD = [None, 0, 1, -5, 1.5, -2.5, 0.0, "12", " 7 ", "abc", "1.5", "", True, False,
         [], {}, [1], {"a": 1}, 999, 1000, 3_000_000]


class Gen:
    def __init__(self, rng: random.Random, mutate: float = 0.0):
        self.rng = rng
        self.mutate = mutate
        self.n = 0

    def uid(self) -> str:
        self.n += 1
        if self.rng.random() < 0.03:
            return self.rng.choice(["", "dup-id", "dup-a", "dup-b"])  # empty / duplicate ids
        return f"{self.n:08X}-0000-4000-8000-{self.rng.randrange(16**12):012X}"

    def tr(self, start: int, dur: int) -> dict:
        return {"start": start, "duration": dur}

    def caption_content(self, text: str):
        r = self.rng.random()
        if r < 0.75:
            return json.dumps({"text": text, "styles": [{"range": [0, len(text)]}]}, ensure_ascii=False)
        return self.rng.choice([text, None, {"text": text}, 5, "[1, 2]", '"quoted"', "  ", "{bad json"])

    def draft(self, depth: int = 0) -> dict:
        rng = self.rng
        fps = rng.choice([30.0, 25.0, 24.0, 60.0, 29.97])
        n_caps = rng.randint(0 if depth else 1, 12)
        t = 0
        texts, text_segs, audios, audio_segs, speeds, fades = [], [], [], [], [], []
        for _ in range(n_caps):
            t += rng.choice([0, 0, 200_000, 1_000_000])
            dur = rng.choice([500_000, 1_000_000, 1_333_333, 2_500_000, 3_000_000])
            mid, sid = self.uid(), self.uid()
            mat = {"id": mid, "type": rng.choice(["subtitle"] * 8 + ["text"]),
                   "content": self.caption_content(rng.choice(TEXTS)), "text_to_audio_ids": []}
            texts.append(mat)
            text_segs.append({"id": sid, "material_id": mid, "target_timerange": self.tr(t, dur),
                              "extra_material_refs": []})
            if rng.random() < 0.5:
                aid, asid, spid = self.uid(), self.uid(), self.uid()
                speed = rng.choice([1.0, 1.0, 1.25, 1.3])
                src_dur = rng.choice([dur, dur - 30_000, dur + 400_000])
                tgt_dur = int(src_dur / speed)
                # boundaries: source ends exactly at, 1 us past, 2 us past the material
                audios.append({"id": aid, "type": "text_to_audio", "text_id": mid,
                               "duration": src_dur + rng.choice([100_000, 100_000, 99_999, 99_998]),
                               "path": rng.choice([f"{PH}/textReading/{aid}.wav", f"textReading/{aid}.wav",
                                                   f"{aid}.wav", ""]),
                               "name": "tts"})
                speeds.append({"id": spid, "type": "speed", "mode": 0, "speed": speed, "curve_speed": None})
                refs = [spid]
                if rng.random() < 0.6:
                    fid = self.uid()
                    fades.append({"id": fid, "type": "audio_fade", "fade_type": 0,
                                  "fade_in_duration": rng.choice([8000, 0, 99_000_000, tgt_dur, tgt_dur + 1, -1]),
                                  "fade_out_duration": 0})
                    refs.append(fid)
                if rng.random() < 0.1:
                    refs.append("missing-ref")
                audio_segs.append({"id": asid, "material_id": aid, "source_timerange": self.tr(100_000, src_dur),
                                   "target_timerange": self.tr(t + rng.choice([0, 0, -300_000]), tgt_dur),
                                   "speed": speed, "is_tone_modify": rng.choice([True, False]),
                                   "extra_material_refs": refs})
                mat["text_to_audio_ids"] = [asid] + (["orphan-seg"] if rng.random() < 0.1 else [])
            t += dur
        total = max(t, 1) + rng.choice([0, 0, 500_000])
        vid, vsid, vspid = self.uid(), self.uid(), self.uid()
        videos = [{"id": vid, "type": "video", "duration": total + rng.choice([0, 5_000_000]),
                   "path": f"{PH}/media/clip.mp4"}]
        speeds.append({"id": vspid, "type": "speed", "mode": 0, "speed": 1.0, "curve_speed": None})
        video_segs = [{"id": vsid, "material_id": vid, "source_timerange": self.tr(0, total),
                       "target_timerange": self.tr(0, total), "speed": 1.0, "extra_material_refs": [vspid]}]
        if rng.random() < 0.5 and total > 4:
            # a second clip with speed so cuts trim sources
            h = total // 2
            video_segs[0]["target_timerange"] = self.tr(0, h)
            video_segs[0]["source_timerange"] = self.tr(0, h)
            vid2, vsid2, sp2 = self.uid(), self.uid(), self.uid()
            videos.append({"id": vid2, "type": "video", "duration": total * 2})
            speeds.append({"id": sp2, "type": "speed", "speed": 2.0})
            video_segs.append({"id": vsid2, "material_id": vid2, "source_timerange": self.tr(1_000, (total - h) * 2),
                               "target_timerange": self.tr(h, total - h), "speed": 2.0, "extra_material_refs": [sp2]})
        drafts = []
        if depth == 0 and rng.random() < 0.3:
            combo_id = self.uid()
            nested = self.draft(depth + 1)
            nested["duration"] = total + rng.choice([0, 0, 1_000, -1_000])
            drafts.append({"id": combo_id, "type": "combination", "draft": nested})
            cvid, cvsid = self.uid(), self.uid()
            videos.append({"id": cvid, "type": "video", "duration": total})
            video_segs.append({"id": cvsid, "material_id": cvid, "source_timerange": self.tr(500, total),
                               "target_timerange": self.tr(0, total), "speed": 1.0,
                               "extra_material_refs": [combo_id]})
            if rng.random() < 0.3:
                drafts.append({"id": "unused-combo", "type": "combination", "draft": {"duration": 5}})
        tracks = [{"id": self.uid(), "type": "video", "segments": video_segs}]
        if text_segs:
            tracks.append({"id": self.uid(), "type": "text", "segments": text_segs})
        if audio_segs:
            tracks.append({"id": self.uid(), "type": "audio", "segments": audio_segs})
            if rng.random() < 0.2:
                tracks.append({"id": self.uid(), "type": "audio", "segments": []})
        if rng.random() < 0.1:
            tracks.append({"id": self.uid(), "type": "text", "segments": [
                {"id": self.uid(), "material_id": "orphan-mat", "target_timerange": self.tr(0, 10)}]})
        d = {
            "id": self.uid(), "name": rng.choice(["", "My Project", "Dự án"]), "duration": total, "fps": fps,
            "version": 360000, "new_version": rng.choice(["110.0.0", None, 5]),
            "canvas_config": {"width": 1920, "height": 1080, "ratio": "original", "background": None},
            "color_space": 0, "config": {"maintrack_adsorb": True}, "platform": {"os": "windows"},
            "update_time": 1700000000, "source": "default",
            "materials": {"texts": texts, "audios": audios, "videos": videos, "speeds": speeds,
                          "audio_fades": fades, "placeholder_infos": [], "beats": [],
                          "drafts": drafts, "canvases": [{"id": self.uid(), "type": "canvas_color"}],
                          "extra_info": {"not": "a list"}},
            "tracks": tracks,
        }
        if fades and rng.random() < 0.1:
            fades.append({"id": "orphan-fade", "type": "audio_fade", "fade_in_duration": 1})
        if self.mutate:
            self._mutate(d)
        # Detach from the shared WEIRD objects so later mutations cannot alias it.
        return json.loads(json.dumps(d, ensure_ascii=False))

    def _mutate(self, obj) -> None:
        """Replace scalars, drop keys, null out containers, in place."""
        rng = self.rng
        if isinstance(obj, dict):
            for k in list(obj):
                v = obj[k]
                r = rng.random()
                if r < self.mutate * 0.2:
                    del obj[k]
                    continue
                if isinstance(v, (dict, list)):
                    if r < self.mutate * 0.4:
                        obj[k] = rng.choice([None, None, None, 5, "s"])
                    else:
                        self._mutate(v)
                elif r < self.mutate:
                    obj[k] = copy.deepcopy(rng.choice(WEIRD))
        elif isinstance(obj, list):
            for i, v in enumerate(obj):
                if isinstance(v, (dict, list)):
                    self._mutate(v)
                elif rng.random() < self.mutate:
                    obj[i] = copy.deepcopy(rng.choice(WEIRD))
