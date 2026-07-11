import os
import sys
import json
import time
import shutil
import subprocess
import random
import traceback
import requests
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Dict, Any, Optional, Callable
from core.logger import logger

from core.config import AppConfig
from core.cache import TtsCache


def _make_chunk_progress(chunk_idx: int, chunk_count: int):
    """Return a progress callback scoped to one parallel chunk."""
    base = chunk_idx / max(chunk_count, 1)
    span = 1.0 / max(chunk_count, 1)
    def callback(stage: str, local_progress: float):
        overall = base + min(max(local_progress, 0.0), 1.0) * span
        logger.debug("TTS chunk %s/%s %s progress: %.3f overall %.3f", chunk_idx + 1, chunk_count, stage, local_progress, overall)
    return callback

class CapCutTtsWrapper:
    def __init__(self, project_name: str = ""):
        self.config = AppConfig()
        self.project_name = project_name
        self.cache = TtsCache()

    def _get_python_executable(self) -> str:
        return sys.executable

    def _get_client_path(self) -> str:
        return os.path.abspath(os.path.join(self.config.capcut_tts_path, "capcut_common_task_client.py"))

    def _get_device_json_path(self) -> str:
        return os.path.abspath(self.config.device_json_path)

    @staticmethodbo
    def _generate_numeric_id(length: int = 19) -> str:
        """Generate a device-style numeric ID with a non-zero first digit."""
        if length < 1:
            raise ValueError("ID length must be at least 1")
        first_digit = str(random.randint(1, 9))
        remaining = "".join(str(random.randint(0, 9)) for _ in range(length - 1))
        return f"{first_digit}{remaining}"

    def _copy_to_project_tts(self, source_path: str | os.PathLike, dest_name: str | None = None) -> Path:
        source = Path(source_path)
        tts_dir = self.config.project_file(self.project_name, "tts", "audio")
        tts_dir.mkdir(parents=True, exist_ok=True)
        dest = tts_dir / (dest_name or source.name)
        if source.resolve() != dest.resolve():
            shutil.copy2(source, dest)
        return dest

    @staticmethod
    def _project_tts_filename(index: int) -> str:
        return f"tts_{int(index):06d}.mp3"

    def _subprocess_env(self) -> dict:
        env = os.environ.copy()
        env["PYTHONIOENCODING"] = "utf-8"
        return env

    def _create_temp_device_json(self) -> tuple[Path, str]:
        device_json = self._get_device_json_path()
        rand_id = self._generate_numeric_id()
        rand_iid = self._generate_numeric_id()

        device_data = {}
        if os.path.exists(device_json):
            try:
                with open(device_json, "r", encoding="utf-8-sig") as f:
                    device_data = json.load(f)
            except Exception as e:
                logger.warning(f"Could not load device.json: {e}")

        device_data["device_id"] = rand_id
        device_data["tdid"] = rand_id
        device_data["iid"] = rand_iid

        temp_device_path = self.config.cache_dir / f"device_{rand_id}.json"
        temp_device_path.parent.mkdir(parents=True, exist_ok=True)
        with open(temp_device_path, "w", encoding="utf-8") as f:
            json.dump(device_data, f, indent=4, ensure_ascii=False)
        return temp_device_path, rand_id

    @staticmethod
    def _parse_client_stdout(stdout: str, context: str) -> tuple[str, str]:
        stdout_lines = stdout.strip().split("\n")
        if len(stdout_lines) < 2:
            raise ValueError(f"Unexpected {context} response from client: {stdout!r}")
        return stdout_lines[0].strip(), stdout_lines[1].strip()

    def _run_client(self, cmd: list[str], env: dict, context: str, check: bool = True) -> subprocess.CompletedProcess:
        result = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", env=env)
        if check and result.returncode != 0:
            raise RuntimeError(
                f"{context} client failed with exit code {result.returncode}. "
                f"stdout={result.stdout!r} stderr={result.stderr!r}"
            )
        return result

    def _download_and_cache(self, speech_url: str, task_id: str, text: str, voice: str, rate: float, resource_id: str, duration: float, subtitle_index: int | None = None) -> Dict[str, Any]:
        download_dir = self.config.project_file(self.project_name, "tts", "temp")
        download_dir.mkdir(parents=True, exist_ok=True)
        temp_download_path = download_dir / f"temp_{task_id}.mp3"

        self._download_file(speech_url, temp_download_path)
        cached_file = self.cache.cache_audio(temp_download_path, text, voice, rate, resource_id, duration=duration)

        if temp_download_path.exists():
            temp_download_path.unlink()

        # Copy from global cache to project directory if project is active
        dest_name = self._project_tts_filename(subtitle_index) if subtitle_index is not None else None
        audio_path = self._copy_to_project_tts(cached_file, dest_name=dest_name) if self.project_name else Path(cached_file)

        return {"audio_path": str(audio_path), "duration": duration, "status": "Downloaded"}

    def generate_tts(self, text: str, voice: Optional[str] = None, resource_id: Optional[str] = None, rate: Optional[float] = None,
                     progress_callback: Optional[Callable[[str, float], None]] = None,
                     is_cancelled_callback: Optional[Callable[[], bool]] = None) -> Optional[Dict[str, Any]]:
        results = self.generate_tts_batch(
            [{"index": 1, "text": text}], voice=voice, resource_id=resource_id, rate=rate,
            progress_callback=progress_callback, is_cancelled_callback=is_cancelled_callback
        )
        return results.get(1)

    def generate_tts_batch(self, items: list[dict], voice: Optional[str] = None, resource_id: Optional[str] = None,
                           rate: Optional[float] = None, progress_callback: Optional[Callable[[str, float], None]] = None,
                           is_cancelled_callback: Optional[Callable[[], bool]] = None,
                           item_completed_callback: Optional[Callable[[int, Dict[str, Any]], None]] = None,
                           use_cache: bool = True) -> Dict[int, Dict[str, Any]]:
        """Generate TTS for many subtitle rows, chunking large batches to avoid rate limits."""
        voice = voice or self.config.default_voice
        resource_id = resource_id or self.config.default_resource_id
        rate = rate if rate is not None else self.config.default_rate

        py_exe = self._get_python_executable()
        client_py = self._get_client_path()
        if not os.path.exists(client_py):
            raise FileNotFoundError(f"capcut_common_task_client.py not found at: {client_py}")

        results: Dict[int, Dict[str, Any]] = {}
        pending: list[dict] = []
        cached_count = 0
        logger.info("TTS batch received %s items. use_cache=%s", len(items), use_cache)

        cache_log_interval = 100
        for item_pos, item in enumerate(items, start=1):
            idx = int(item["index"])
            text = str(item.get("text", ""))
            cached_file, cached_duration = (
                self.cache.get_cached_file(text, voice, rate, resource_id)
                if use_cache
                else (None, None)
            )
            if cached_file:
                dest_name = self._project_tts_filename(idx)
                audio_path = self._copy_to_project_tts(cached_file, dest_name=dest_name) if self.project_name else Path(cached_file)
                # Use cached duration; fall back to ffprobe only if metadata is missing
                duration = cached_duration if cached_duration is not None else self._get_audio_duration(audio_path)
                res_dict = {"audio_path": str(audio_path), "duration": duration, "status": "Cached"}
                results[idx] = res_dict
                cached_count += 1
                if item_completed_callback:
                    try:
                        item_completed_callback(idx, res_dict)
                    except Exception as cb_err:
                        logger.error(f"Error in item_completed_callback for cached item {idx}: {cb_err}")
                if cached_count % cache_log_interval == 0 or item_pos == len(items):
                    logger.info(
                        "TTS cache attach progress: %s/%s cache hits applied, %s items checked.",
                        cached_count,
                        len(items),
                        item_pos,
                    )
            elif text.strip():
                pending.append({"index": idx, "text": text})

        if use_cache:
            logger.info(f"TTS cache: {cached_count} items hit, {len(pending)} items pending generation.")

        if not pending:
            return results

        # Chunk large batches to avoid rate limiting without duplicating the
        # pending list in memory for large projects.
        chunk_size = max(1, int(self.config.get("tts_chunk_size", 15) or 15))
        parallel_chunks = max(1, int(self.config.get("tts_parallel_chunks", 2) or 2))
        chunk_count = (len(pending) + chunk_size - 1) // chunk_size
        logger.info(
            "Splitting %s pending items into %s chunks of max %s; parallel_chunks=%s",
            len(pending),
            chunk_count,
            chunk_size,
            parallel_chunks,
        )

        pending_chunks = [
            pending[chunk_idx * chunk_size:(chunk_idx + 1) * chunk_size]
            for chunk_idx in range(chunk_count)
        ]

        # Diagnostic: confirm the chunk partition is well-formed before any
        # submission. An empty last chunk would silently skip processing for
        # tail items; an empty inner chunk would leave a gap in coverage.
        empty_chunks = [i + 1 for i, c in enumerate(pending_chunks) if not c]
        if empty_chunks:
            logger.error(
                "TTS chunk partition produced empty chunks (positions: %s); "
                "total chunks=%s, chunk_size=%s, pending=%s",
                empty_chunks, chunk_count, chunk_size, len(pending),
            )
        logger.info(
            "TTS chunk plan: total_chunks=%s, sizes=%s, parallel_chunks=%s, total_items=%s",
            chunk_count,
            [len(c) for c in pending_chunks],
            parallel_chunks,
            len(pending),
        )

        if parallel_chunks > 1 and chunk_count > 1:
            with ThreadPoolExecutor(max_workers=min(parallel_chunks, chunk_count)) as executor:
                future_to_chunk = {}
                for chunk_idx, chunk in enumerate(pending_chunks):
                    if is_cancelled_callback and is_cancelled_callback():
                        logger.info("TTS batch was cancelled by user.")
                        return results
                    if not chunk:
                        logger.error("Skipping empty TTS chunk %s/%s", chunk_idx + 1, chunk_count)
                        continue
                    logger.info(
                        "Submitting TTS chunk %s/%s (%s items) to parallel worker.",
                        chunk_idx + 1,
                        chunk_count,
                        len(chunk),
                    )
                    future = executor.submit(
                        self._process_single_chunk,
                        chunk,
                        voice,
                        resource_id,
                        rate,
                        py_exe,
                        client_py,
                        _make_chunk_progress(chunk_idx, chunk_count),
                        is_cancelled_callback,
                        item_completed_callback,
                        items,
                        results,
                    )
                    future_to_chunk[future] = (chunk_idx, chunk)

                for future in as_completed(future_to_chunk):
                    chunk_idx, chunk = future_to_chunk[future]
                    if is_cancelled_callback and is_cancelled_callback():
                        logger.info("TTS batch was cancelled by user.")
                        return results
                    try:
                        chunk_results = future.result()
                    except Exception as e:
                        # Full traceback so a silent exception between chunk
                        # submissions cannot make the loop appear to "stop".
                        logger.error(
                            "TTS chunk %s/%s failed in parallel worker: %s\n%s",
                            chunk_idx + 1, chunk_count, e, traceback.format_exc(),
                        )
                        continue

                    logger.info(
                        "TTS chunk %s/%s completed: %s/%s items.",
                        chunk_idx + 1,
                        chunk_count,
                        len(chunk_results),
                        len(chunk),
                    )
                    results.update(chunk_results)
                    # NOTE: chunk results are NOT re-broadcast via item_completed_callback
                    # here because _process_single_chunk already fired it for each item
                    # during download/cache. Re-firing would double-count and cause the
                    # GUI progress bar to exceed 100% on parallel batches.
            return results

        for chunk_idx, chunk in enumerate(pending_chunks):
            if is_cancelled_callback and is_cancelled_callback():
                logger.info("TTS batch was cancelled by user.")
                return results

            logger.info(f"Processing chunk {chunk_idx + 1}/{chunk_count} ({len(chunk)} items)")
            chunk_results = self._process_single_chunk(
                chunk, voice, resource_id, rate, py_exe, client_py,
                progress_callback, is_cancelled_callback, item_completed_callback,
                items, results
            )
            results.update(chunk_results)

            if chunk_idx < chunk_count - 1:
                time.sleep(0.5)

        return results

    def _process_single_chunk(self, pending: list[dict], voice: str, resource_id: str, rate: float,
                              py_exe: str, client_py: str,
                              progress_callback: Optional[Callable[[str, float], None]],
                              is_cancelled_callback: Optional[Callable[[], bool]],
                              item_completed_callback: Optional[Callable[[int, Dict[str, Any]], None]],
                              all_items: list[dict], all_results: Dict[int, Dict[str, Any]]) -> Dict[int, Dict[str, Any]]:
        """Process a single chunk of TTS items."""
        results: Dict[int, Dict[str, Any]] = {}
        temp_device_path = None
        env = self._subprocess_env()
        chunk_started_at = time.perf_counter()

        try:
            temp_device_path, rand_id = self._create_temp_device_json()
            cmd_new = [py_exe, client_py, "tts-new", "--device-json", str(temp_device_path), "--voice", voice,
                       "--resource-id", resource_id, "--rate", str(rate)]
            for item in pending:
                cmd_new.extend(["--text", item["text"]])

            logger.info("Submitting %s TTS tasks with randomized device ID: %s", len(pending), rand_id)
            if progress_callback:
                progress_callback("Submitting", 0.1)

            submit_started_at = time.perf_counter()
            res = self._run_client(cmd_new, env, "TTS submission")
            logger.info(
                "TTS chunk submit finished in %.2fs for %s items.",
                time.perf_counter() - submit_started_at,
                len(pending),
            )
            status_code, resp_body_str = self._parse_client_stdout(res.stdout, "TTS submission")
            if status_code != "200":
                raise ValueError(f"HTTP Status {status_code}: {resp_body_str}")

            resp_json = json.loads(resp_body_str)
            if str(resp_json.get("ret", "")) != "0":
                raise ValueError(f"API Error {resp_json.get('ret')}: {resp_json.get('errmsg')} | raw={resp_body_str}")

            tasks = resp_json.get("data", {}).get("tasks", [])
            if not tasks:
                raise ValueError(f"Missing tasks in response: {resp_body_str}")

            task_data = tasks[0]
            task_id = task_data.get("id")
            token = task_data.get("token")
            if not task_id or not token:
                raise ValueError(f"Missing task id/token in response: {resp_body_str}")

            base_poll_interval = max(0.3, float(self.config.get("tts_poll_interval_sec", 1.0) or 1.0))
            max_polls = max(1, int(300 / base_poll_interval))
            poll_interval = base_poll_interval
            for poll_count in range(1, max_polls + 1):
                if is_cancelled_callback and is_cancelled_callback():
                    logger.info("TTS batch was cancelled by user.")
                    return results

                time.sleep(poll_interval)
                # Adaptive backoff: poll nhanh ban dau, cham dan khi cho lau.
                poll_interval = min(poll_interval * 1.15, 3.0)
                if progress_callback:
                    progress_callback("Generating", 0.1 + (poll_count / max_polls) * 0.7)

                query_result = self._query_one_task(py_exe, client_py, env, task_id, token)
                if query_result is None:
                    continue

                audio_subtitles = query_result
                logger.info(
                    "TTS chunk server generation finished after %.2fs and %s poll(s).",
                    time.perf_counter() - submit_started_at,
                    poll_count,
                )
                if len(audio_subtitles) < len(pending):
                    logger.warning(
                        "TTS task %s returned %s audio segments for %s pending subtitles.",
                        task_id,
                        len(audio_subtitles),
                        len(pending),
                    )

                # Parallel download all audio files (5 concurrent workers)
                from concurrent.futures import ThreadPoolExecutor, as_completed
                
                # Strict length check: the server should return one audio per pending
                # item. If it under-delivers, mark every missing index as Failed so the
                # caller learns about the gap instead of silently losing subtitles.
                if len(audio_subtitles) < len(pending):
                    missing_indices = [
                        pending[i]["index"]
                        for i in range(len(audio_subtitles), len(pending))
                    ]
                    logger.error(
                        "TTS task %s returned %s audio segments for %s pending subtitles; "
                        "marking %s missing indices as Failed: %s",
                        task_id,
                        len(audio_subtitles),
                        len(pending),
                        len(missing_indices),
                        missing_indices,
                    )
                    for missing_idx in missing_indices:
                        failed_res = {
                            "audio_path": "",
                            "duration": 0.0,
                            "status": "Failed",
                            "error": "Server did not return audio for this subtitle",
                        }
                        results[missing_idx] = failed_res
                        if item_completed_callback:
                            try:
                                item_completed_callback(missing_idx, failed_res)
                            except Exception as cb_err:
                                logger.error(
                                    "Error in item_completed_callback for failed item %s: %s",
                                    missing_idx, cb_err,
                                )

                download_tasks = []
                for item, speech_info in zip(pending, audio_subtitles):
                    idx = item["index"]
                    speech_url = speech_info.get("speech_url")
                    duration = float(speech_info.get("duration", 0)) / 1000.0
                    if not speech_url:
                        logger.error("Missing speech_url for subtitle #%s in task %s", idx, task_id)
                        failed_res = {
                            "audio_path": "",
                            "duration": 0.0,
                            "status": "Failed",
                            "error": "Missing speech_url in server response",
                        }
                        results[idx] = failed_res
                        if item_completed_callback:
                            try:
                                item_completed_callback(idx, failed_res)
                            except Exception as cb_err:
                                logger.error(
                                    "Error in item_completed_callback for failed item %s: %s",
                                    idx, cb_err,
                                )
                        continue

                    segment_task_id = f"{task_id}_{idx}"
                    download_tasks.append({
                        "item": item,
                        "url": speech_url,
                        "task_id": segment_task_id,
                        "duration": duration
                    })

                download_started_at = time.perf_counter()
                download_workers = max(1, int(self.config.get("tts_download_workers", 8) or 8))
                with ThreadPoolExecutor(max_workers=download_workers) as executor:
                    future_to_task = {}
                    for task in download_tasks:
                        future = executor.submit(
                            self._download_and_cache,
                            task["url"],
                            task["task_id"],
                            task["item"]["text"],
                            voice,
                            rate,
                            resource_id,
                            task["duration"],
                            task["item"]["index"],
                        )
                        future_to_task[future] = task

                    cancelled = False
                    for future in as_completed(future_to_task):
                        # Honor cancellation while downloads are in flight; the `with`
                        # block will let any in-flight worker finish before returning,
                        # which is bounded by `download_workers` concurrent tasks.
                        if is_cancelled_callback and is_cancelled_callback():
                            logger.info("TTS batch cancelled by user during download.")
                            cancelled = True
                            break
                        task = future_to_task[future]
                        idx = task["item"]["index"]
                        try:
                            res_dict = future.result()
                            results[idx] = res_dict
                            logger.info("TTS task %s segment for subtitle #%s succeeded. Duration: %ss",
                                      task["task_id"], idx, task["duration"])

                            if item_completed_callback:
                                try:
                                    item_completed_callback(idx, res_dict)
                                except Exception as cb_err:
                                    logger.error(f"Error in item_completed_callback for item {idx}: {cb_err}")

                            if progress_callback:
                                combined_count = len(all_results) + len(results)
                                progress_callback("Downloading", 0.8 + (combined_count / max(len(all_items), 1)) * 0.2)
                        except Exception as e:
                            logger.error(f"Download failed for subtitle #{idx}: {e}")
                            failed_res = {
                                "audio_path": "",
                                "duration": 0.0,
                                "status": "Failed",
                                "error": str(e),
                            }
                            results[idx] = failed_res
                            if item_completed_callback:
                                try:
                                    item_completed_callback(idx, failed_res)
                                except Exception as cb_err:
                                    logger.error(
                                        "Error in item_completed_callback for failed item %s: %s",
                                        idx, cb_err,
                                    )

                    if cancelled:
                        # Surface every still-pending download as Failed so the caller
                        # learns about them; the executor `with` exit will drain the
                        # in-flight workers on its own.
                        for future, task in future_to_task.items():
                            idx = task["item"]["index"]
                            if idx in results:
                                continue
                            failed_res = {
                                "audio_path": "",
                                "duration": 0.0,
                                "status": "Failed",
                                "error": "Cancelled by user",
                            }
                            results[idx] = failed_res
                            if item_completed_callback:
                                try:
                                    item_completed_callback(idx, failed_res)
                                except Exception as cb_err:
                                    logger.error(
                                        "Error in item_completed_callback for cancelled item %s: %s",
                                        idx, cb_err,
                                    )
                        logger.info(
                            "TTS chunk download cancelled: %s/%s items resolved before cancel.",
                            len(results),
                            len(download_tasks),
                        )

                logger.info(
                    "TTS chunk download/cache finished in %.2fs with %s/%s successful item(s); total chunk time %.2fs.",
                    time.perf_counter() - download_started_at,
                    len(results),
                    len(download_tasks),
                    time.perf_counter() - chunk_started_at,
                )
                return results

            logger.error("TTS batch polling timed out for task %s", task_id)
            return results
        except Exception as e:
            logger.error(f"TTS chunk failed: {e}")
            return results
        finally:
            if temp_device_path and temp_device_path.exists():
                try:
                    temp_device_path.unlink()
                    logger.info(f"Cleaned up temporary randomized device config: {temp_device_path}")
                except Exception as cleanup_err:
                    logger.warning(f"Could not clean up temporary device config {temp_device_path}: {cleanup_err}")

    def _query_one_task(self, py_exe: str, client_py: str, env: dict, task_id: str, token: str) -> Optional[list[dict]]:
        cmd_query = [py_exe, client_py, "tts-query", "--task-id", task_id, "--token", token]
        res_q = self._run_client(cmd_query, env, "TTS query", check=False)
        if res_q.returncode != 0:
            logger.error("TTS query client failed with exit code %s. stdout=%r stderr=%r", res_q.returncode, res_q.stdout, res_q.stderr)
            return None

        status_q, resp_q_str = self._parse_client_stdout(res_q.stdout, "TTS query")
        if status_q != "200":
            logger.warning(f"Poll HTTP Status {status_q}: {resp_q_str}")
            return None

        resp_q_json = json.loads(resp_q_str)
        if str(resp_q_json.get("ret", "")) != "0":
            raise ValueError(f"Poll API Error {resp_q_json.get('ret')}: {resp_q_json.get('errmsg')} | raw={resp_q_str}")

        task_list = resp_q_json.get("data", {}).get("tasks", [])
        if not task_list:
            return None

        task_info = task_list[0]
        status = task_info.get("status")
        if status in {"queueing", "running", "processing"}:
            return None
        if status in {"failed", "cancelled"}:
            raise ValueError(f"CapCut TTS task {task_id} failed on server side: {task_info}")
        if status != "succeed":
            return None

        payload_str = task_info.get("payload")
        if not payload_str:
            raise ValueError("Task status is succeed, but payload string is empty.")

        payload_json = json.loads(payload_str)
        audio_subtitles = payload_json.get("audio_subtitles", [])
        if not audio_subtitles:
            raise ValueError(f"Missing audio_subtitles in payload: {payload_str}")
        return audio_subtitles

    def _download_file(self, url: str, dest_path: Path):
        r = requests.get(url, stream=True, timeout=30)
        r.raise_for_status()
        with open(dest_path, "wb") as f:
            for chunk in r.iter_content(chunk_size=8192):
                f.write(chunk)

    def _get_audio_duration(self, file_path: Path) -> float:
        ffprobe_bin = self.config.ffprobe_path
        cmd = [ffprobe_bin, "-v", "error", "-show_entries", "format=duration", "-of", "json", str(file_path)]
        try:
            res = subprocess.run(cmd, capture_output=True, text=True, check=True)
            data = json.loads(res.stdout)
            return float(data.get("format", {}).get("duration", 0.0))
        except Exception as e:
            logger.error(f"Could not read duration of cached file {file_path}: {e}")
            return 0.0