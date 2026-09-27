"""Pinned optional Laya endpoint; reject state truncation before inference."""
import importlib.metadata
import os


def require_complete_state(context):
    from laya.common import build_sequence

    agent = context.agent
    limit = agent.cfg.get("max_len", 512)
    head_limit = agent.cfg.get("head_max_len", 192)
    for name, question in context.questions.items():
        agent._check_question(name, question)
        internal = agent._to_internal(question)
        for state in context.states:
            sequence, _ = build_sequence(
                agent.tok, state, internal, max_len=limit + 1,
                head_max_len=head_limit,
            )
            if len(sequence) > limit:
                raise ValueError("classification context exceeds the complete-state token limit")


def create_app():
    if importlib.metadata.version("laya") != "0.3.20":
        raise RuntimeError("this endpoint requires laya 0.3.20")
    if not os.environ.get("LAYA_API_KEY"):
        raise RuntimeError("LAYA_API_KEY is required")
    model_path = os.environ["LAYA_MODEL_PATH"]
    if not os.path.isabs(model_path):
        raise RuntimeError("LAYA_MODEL_PATH must be an absolute local checkpoint path")
    from laya.router import Router
    from laya.serve import _apply_thread_limit, create_app as upstream_app

    _apply_thread_limit()
    router = Router(
        device="cpu", default="multilingual", max_loaded=1,
        on_predict_start=require_complete_state,
    )
    # Router merges supplied models with defaults; explicitly restrict this endpoint.
    router.models = {"multilingual": model_path}
    router.preload(["multilingual"])
    return upstream_app(router)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(create_app(), host="0.0.0.0", port=8000)
