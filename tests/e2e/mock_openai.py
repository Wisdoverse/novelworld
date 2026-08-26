#!/usr/bin/env python3
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


ANCHOR = "林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。"
ENDING = "北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。"
CONTROL_LOCK = threading.Lock()
DELAYS_MS = {}
FAILURES_REMAINING = {"canon": 1, "narrative_transition": 3, "world_turn": 0}
CALLS = {}
ACTIVE = {}
PEAK = {}


def operation_for(path, request, prompt=""):
    if path == "/v1/images/generations":
        return "image"
    if path == "/v1/embeddings":
        return "embedding"
    if request.get("stream"):
        return "stream"
    for marker, operation in (
        ("source-backed canonical facts", "canon"),
        ("提取所有重要角色信息", "characters"),
        ("提取角色和角色关系", "character_chunk"),
        ("找出 2-5 个玩家", "nodes"),
        ("anchor_quote", "branch"),
        ("You propose one bounded world transition", "world_turn"),
        ("You generate one structured transition", "narrative_transition"),
        ("玩家时间线主笔", "player_chapter"),
        ("对话摘要助手", "summary"),
    ):
        if marker in prompt:
            return operation
    return "chat"


def start_operation(operation):
    with CONTROL_LOCK:
        CALLS[operation] = CALLS.get(operation, 0) + 1
        ACTIVE[operation] = ACTIVE.get(operation, 0) + 1
        PEAK[operation] = max(PEAK.get(operation, 0), ACTIVE[operation])
        return DELAYS_MS.get(operation, DELAYS_MS.get("default", 0))


def finish_operation(operation):
    with CONTROL_LOCK:
        ACTIVE[operation] -= 1


def consume_failure(operation):
    with CONTROL_LOCK:
        if FAILURES_REMAINING.get(operation, 0) == 0:
            return False
        FAILURES_REMAINING[operation] -= 1
        return True


def control_snapshot():
    with CONTROL_LOCK:
        return {
            "calls": dict(CALLS),
            "active": dict(ACTIVE),
            "peak": dict(PEAK),
            "delays_ms": dict(DELAYS_MS),
            "failures_remaining": dict(FAILURES_REMAINING),
        }


def committed_world_context(request):
    marker = "## 已提交开放世界上下文\n"
    for message in request.get("messages", []):
        content = message.get("content", "")
        if message.get("role") == "system" and content.startswith(marker):
            try:
                return json.loads(content.rsplit("\n", 1)[1])
            except (json.JSONDecodeError, TypeError):
                return None
    return None


def reset_control(request):
    delays = request.get("delays_ms", {})
    failures = request.get("failures_remaining", {})
    if not isinstance(delays, dict) or not isinstance(failures, dict):
        raise ValueError("control values must be objects")
    if any(
        not isinstance(key, str)
        or not isinstance(value, int)
        or isinstance(value, bool)
        or not 0 <= value <= 10_000
        for key, value in delays.items()
    ):
        raise ValueError("delays must be integer milliseconds from 0 to 10000")
    if any(
        key not in FAILURES_REMAINING
        or not isinstance(value, int)
        or isinstance(value, bool)
        or not 0 <= value <= 100
        for key, value in failures.items()
    ):
        raise ValueError("invalid failure controls")
    with CONTROL_LOCK:
        if any(ACTIVE.values()):
            raise RuntimeError("provider requests are still active")
        DELAYS_MS.clear()
        DELAYS_MS.update(delays)
        for key in FAILURES_REMAINING:
            FAILURES_REMAINING[key] = failures.get(key, 0)
        CALLS.clear()
        ACTIVE.clear()
        PEAK.clear()
    return control_snapshot()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            return self.json_response({"status": "ok"})
        if self.path == "/__control__/stats":
            return self.json_response(control_snapshot())
        return self.json_response({"error": "not found"}, 404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        request = json.loads(self.rfile.read(length) or b"{}")

        if self.path == "/__control__/reset":
            try:
                return self.json_response(reset_control(request))
            except (RuntimeError, ValueError) as error:
                return self.json_response({"error": str(error)}, 409)

        prompt = "\n".join(message.get("content", "") for message in request.get("messages", []))
        operation = operation_for(self.path, request, prompt)
        delay_ms = start_operation(operation)
        try:
            if delay_ms:
                time.sleep(delay_ms / 1000)
            return self.provider_response(request, prompt)
        finally:
            finish_operation(operation)

    def provider_response(self, request, prompt):
        if self.path == "/v1/images/generations":
            return self.json_response({"data": [{"url": "https://example.invalid/lin-lan.png"}]})
        if self.path == "/v1/embeddings":
            return self.json_response({"data": [{"embedding": [0.0] * 1536}], "model": "e2e"})
        if self.path != "/v1/chat/completions":
            return self.json_response({"error": {"message": "not found"}}, 404)
        if request.get("stream"):
            assert request.get("stream_options") == {"include_usage": True}
            world_context = committed_world_context(request)
            reply = (
                "林岚知道你已经改变了两回合的世界，也会依照自己的目标继续调查。"
                if (
                    world_context is not None
                    and world_context.get("turn_number") == 2
                    and world_context.get("world_time") == 2
                    and world_context.get("recent_actions") == []
                    and world_context.get("recent_player_events")
                    == [
                        {
                            "turn_number": 1,
                            "world_time": 1,
                            "summary": "云舟调查北塔换防",
                        },
                        {
                            "turn_number": 2,
                            "world_time": 2,
                            "summary": "云舟整理地下回廊线索",
                        },
                    ]
                )
                else "林岚记得你，也愿意继续同行。"
            )
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for payload in (
                {"choices": [{"delta": {"content": reply}, "finish_reason": None}]},
                {"choices": [{"delta": {}, "finish_reason": "stop"}]},
                {"choices": [], "usage": {"prompt_tokens": 4, "completion_tokens": 2, "prompt_cache_hit_tokens": 1}},
            ):
                self.wfile.write(f"data: {json.dumps(payload, ensure_ascii=False)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            return

        content = self.response_for(prompt)
        self.json_response({
            "choices": [{"message": {"content": content}}],
            "model": "e2e",
            "usage": {"prompt_tokens": 4, "completion_tokens": 2, "prompt_cache_hit_tokens": 1},
        })

    @staticmethod
    def response_for(prompt):
        if "source-backed canonical facts" in prompt:
            if consume_failure("canon"):
                return "{}"
            final_chunk = "FINAL_CHUNK: true" in prompt
            excerpt = ENDING if final_chunk else ANCHOR
            return json.dumps({
                "arc": {
                    "key": "north-tower-investigation",
                    "title": "北塔调查",
                    "summary": "林岚沿线索进入北塔。",
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                },
                "events": [{
                    "summary": "林岚推进北塔调查。",
                    "caused_by": [],
                    "locations": ["北塔"],
                    "characters": ["林岚"],
                    "factions": [],
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                }],
                "locations": [{
                    "name": "北塔",
                    "description": "风暴中的古老高塔。",
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                }],
                "factions": [],
                "world_rules": [],
                "character_goals": [{
                    "character": "林岚",
                    "description": "寻找失踪的守门人。",
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                }],
                "character_states": [{
                    "name": "林岚",
                    "state": "继续调查北塔与守门人的去向。",
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                }],
                "relationships": [],
                "deaths": [],
                "threads": [{
                    "key": "missing-gatekeeper",
                    "description": "守门人的去向仍未确定。",
                    "status": "open",
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                }],
                "ending": ({
                    "summary": "北塔暗门开启，守门人的去向仍待确认。",
                    "faction_states": [],
                    "location_states": [{
                        "name": "北塔",
                        "state": "地下回廊已经开启。",
                        "evidence": {"excerpt": excerpt, "confidence": 1.0},
                    }],
                    "evidence": {"excerpt": excerpt, "confidence": 1.0},
                } if final_chunk else None),
            }, ensure_ascii=False)
        if "提取所有重要角色信息" in prompt:
            return json.dumps({
                "characters": [{
                    "name": "林岚",
                    "aliases": [],
                    "role": "protagonist",
                    "description": "寻找失踪守门人的年轻旅者。",
                    "personality": "谨慎、坚定、重视承诺。",
                    "background": "来自北境边城，熟悉古塔传说。",
                    "speaking_style": "语气沉静，表达直接。",
                    "appearance": "黑发灰眼，身穿深蓝旅行斗篷。",
                    "first_appearance_chapter": 1,
                }],
                "relationships": [],
                "world_summary": "风暴笼罩的北境中，古塔与失踪的守门人牵动着边城命运。",
                "genre": "奇幻",
            }, ensure_ascii=False)
        if "提取角色和角色关系" in prompt:
            return json.dumps({
                "characters": [{
                    "name": "林岚",
                    "aliases": [],
                    "role": "protagonist",
                    "description": "寻找失踪守门人的年轻旅者。",
                    "personality": "谨慎、坚定、重视承诺。",
                    "background": "来自北境边城，熟悉古塔传说。",
                    "speaking_style": "语气沉静，表达直接。",
                    "appearance": "黑发灰眼，身穿深蓝旅行斗篷。",
                    "first_appearance_chapter": 1,
                }],
                "relationships": [],
            }, ensure_ascii=False)
        if "找出 2-5 个玩家" in prompt:
            return json.dumps({"nodes": [{
                "chapter_number": 1,
                "description": "北塔风暴逼近，玩家必须决定如何协助林岚寻找守门人。",
                "choices": [
                    {"text": "与林岚立即前往北塔", "hint": "风暴中藏着线索……"},
                    {"text": "先向边城居民调查", "hint": "旧传闻可能并非虚构……"},
                ],
            }, {
                "chapter_number": 2,
                "description": "北塔石门缓缓开启，玩家必须决定如何回应塔内的异动。",
                "choices": [
                    {"text": "跟随林岚进入回廊", "hint": "铜铃标记着一条隐秘路线……"},
                    {"text": "留在塔门观察足迹", "hint": "来者或许仍藏在风暴之中……"},
                ],
            }]}, ensure_ascii=False)
        if "anchor_quote" in prompt:
            return json.dumps({
                "anchor_quote": ANCHOR,
                "description": "风暴正在吞没北塔，留给你和林岚的时间已经不多。",
                "choices": [
                    {"text": "与林岚立即前往北塔", "hint": "塔门之后危机四伏……"},
                    {"text": "先向边城居民调查", "hint": "有人隐瞒了旧日真相……"},
                ],
            }, ensure_ascii=False)
        if "You propose one bounded world transition" in prompt:
            if consume_failure("world_turn"):
                return "{}"
            session = json.loads(prompt.split("WORLD_SESSION: ", 1)[1].split("\nWORLD_STATE:", 1)[0])
            recent_turns = json.loads(prompt.split("RECENT_TURNS: ", 1)[1])
            context = session["entry_context"]
            current_event = next((
                event for event in session["canonical_events"]
                if event["status"] in ("scheduled", "delayed")
            ), None)
            first_turn = session["turn_number"] == 0
            if first_turn and recent_turns:
                return "{}"
            if not first_turn:
                previous = recent_turns[-1] if recent_turns else {}
                if (
                    previous.get("turn_number") != session["turn_number"]
                    or (
                        session["turn_number"] == 1
                        and (
                            previous.get("action", {}).get("intent")
                            != "查清北塔换防并阻止伏击"
                            or not previous.get("rendered_narrative", "").endswith(
                                "林岚仍按自己的目标追查守门人，原定围堵因此受阻。"
                            )
                        )
                    )
                ):
                    return "{}"
            return json.dumps({
                "schema_version": 1,
                "rendered_narrative": (
                    "云舟沿北塔外墙查清换防规律，在不替林岚作决定的前提下封住了伏击通道。林岚仍按自己的目标追查守门人，原定围堵因此受阻。"
                    if first_turn else
                    "云舟整理两回合积累的线索，决定继续绘制地下回廊。林岚独自核对铜铃上的刻痕，两人的行动在同一世界里彼此印证。"
                ),
                "events": [{
                    "summary": "云舟调查北塔换防" if first_turn else "云舟整理地下回廊线索",
                    # 林岚在两段叙事中都有明确、独立的见证/行动；this
                    # explicit provenance permits the event summaries, not the
                    # player's private action intent, to enter her chat context.
                    "actor_character_ids": [context["characters"][0]["id"]],
                    "location_id": context["locations"][0]["id"],
                }],
                "relationship_changes": ([{
                    "character_id": context["characters"][0]["id"],
                    "delta": 2,
                    "reason": "林岚看见云舟独立完成了调查",
                }] if first_turn else []),
                "location_changes": [],
                "thread_changes": [],
                "player_location_id": None,
                "inventory_additions": [],
                "inventory_removals": [],
                "knowledge_discoveries": ["北塔换防规律"] if first_turn else [],
                "faction_changes": [],
                "canonical_event_change": ({
                    "event_id": current_event["id"],
                    "status": "obstructed",
                    "reason": "玩家封住伏击通道，但没有控制任何原著角色",
                } if first_turn and current_event else None),
            }, ensure_ascii=False)
        if "You generate one structured transition" in prompt:
            if consume_failure("narrative_transition"):
                return "{}"
            canon = json.loads(prompt.split("CANON_CONTEXT:\n", 1)[1].split("\nWORLD_STATE:", 1)[0])
            character_id = canon["characters"][0]["id"]
            location_id = canon["locations"][0]["id"]
            thread_id = canon["threads"][0]["id"]
            return json.dumps({
                "schema_version": 1,
                "rendered_narrative": "你与林岚踏入风暴，沿着旧地图找到北塔暗门。守门人的灯仍在风中闪烁，新的脚印却通向地下。",
                "events": [{
                    "summary": "你与林岚找到北塔暗门",
                    "actor_character_ids": [character_id],
                    "location_id": location_id,
                }],
                "relationship_changes": [{
                    "character_id": character_id,
                    "delta": 5,
                    "reason": "共同进入风暴",
                }],
                "location_changes": [{
                    "location_id": location_id,
                    "state": "暗门已经开启",
                    "reason": "玩家与林岚找到了入口",
                }],
                "thread_changes": [{
                    "thread_id": thread_id,
                    "status": "open",
                    "description": "守门人的去向仍待确认",
                }],
            }, ensure_ascii=False)
        if "玩家时间线主笔" in prompt:
            return "你和林岚沿暗门进入北塔深处。守门人留下的铜铃突然响起，墙后传来低沉回应，新的道路由此展开。"
        return "林岚记得你，也愿意继续同行。"

    def json_response(self, payload, status=200):
        body = json.dumps(payload, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


class LlmServer(ThreadingHTTPServer):
    # Capacity tests intentionally open more than socketserver's default five
    # queued connections; the stub must not become the measured bottleneck.
    request_queue_size = 128
    daemon_threads = True


if __name__ == "__main__":
    LlmServer(("0.0.0.0", 18080), Handler).serve_forever()
