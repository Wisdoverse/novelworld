#!/usr/bin/env python3
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


ANCHOR = "林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200 if self.path == "/health" else 404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        request = json.loads(self.rfile.read(length) or b"{}")

        if self.path == "/v1/images/generations":
            return self.json_response({"data": [{"url": "https://example.invalid/lin-lan.png"}]})
        if self.path == "/v1/embeddings":
            return self.json_response({"data": [{"embedding": [0.0] * 1536}], "model": "e2e"})
        if self.path != "/v1/chat/completions":
            return self.json_response({"error": {"message": "not found"}}, 404)

        prompt = "\n".join(message.get("content", "") for message in request.get("messages", []))
        if request.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for payload in (
                {"choices": [{"delta": {"content": "林岚记得你，也愿意继续同行。"}, "finish_reason": None}]},
                {"choices": [{"delta": {}, "finish_reason": "stop"}]},
            ):
                self.wfile.write(f"data: {json.dumps(payload, ensure_ascii=False)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            return

        content = self.response_for(prompt)
        self.json_response({
            "choices": [{"message": {"content": content}}],
            "model": "e2e",
            "usage": {"prompt_tokens": 1, "completion_tokens": 1},
        })

    @staticmethod
    def response_for(prompt):
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
        if "找出 2-5 个玩家" in prompt:
            return json.dumps({"nodes": [{
                "chapter_number": 1,
                "description": "北塔风暴逼近，玩家必须决定如何协助林岚寻找守门人。",
                "choices": [
                    {"text": "与林岚立即前往北塔", "hint": "风暴中藏着线索……"},
                    {"text": "先向边城居民调查", "hint": "旧传闻可能并非虚构……"},
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
        if "生成行动后的故事发展" in prompt:
            return "你与林岚踏入风暴，沿着旧地图找到北塔暗门。守门人的灯仍在风中闪烁，新的脚印却通向地下。"
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


if __name__ == "__main__":
    ThreadingHTTPServer(("0.0.0.0", 18080), Handler).serve_forever()
