"""Run with the pinned Laya image; no weights, provider, or server needed."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('novelworld_laya', Path(__file__).with_name('serve.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Tokenizer:
    mask_token = '[MASK]'
    mask_token_id = 1
    cls_token_id = 2
    sep_token_id = 3

    def __call__(self, text, **options):
        tokens = list(range(len(text)))
        if options.get('truncation'):
            tokens = tokens[:options['max_length']]
        return {'input_ids': tokens}


class CompleteStateTest(unittest.TestCase):
    def context(self, state, questions=None):
        from laya.agent import Agent
        agent = SimpleNamespace(
            cfg={'max_len': 1024, 'head_max_len': 256}, tok=Tokenizer(),
            _check_question=Agent._check_question,
            _to_internal=Agent._to_internal,
        )
        return SimpleNamespace(agent=agent, states=[state], questions=questions or {
            'series': {'type': 'choice', 'instructions': 'match', 'criteria': {'A': 'same', 'U': 'unknown'}},
        })

    def test_accepts_exact_limit_and_rejects_one_more_token(self):
        from laya.common import build_sequence
        context = self.context('')
        q = context.agent._to_internal(context.questions['series'])
        head, _ = build_sequence(context.agent.tok, '', q, max_len=1025, head_max_len=256)
        room = 1024 - len(head)
        module.require_complete_state(self.context('x' * room))
        with self.assertRaises(ValueError):
            module.require_complete_state(self.context('x' * (room + 1)))

    def test_checks_each_question_and_rejects_invalid_schema(self):
        questions = {
            'short': {'type': 'choice', 'instructions': '', 'criteria': {'A': 'a', 'U': 'u'}},
            'long': {'type': 'choice', 'instructions': 'x' * 240, 'criteria': {'A': 'a', 'U': 'u'}},
        }
        with self.assertRaises(ValueError):
            module.require_complete_state(self.context('x' * 800, questions))
        with self.assertRaises(ValueError):
            module.require_complete_state(self.context('', {'bad': {'type': 'bogus'}}))

    def test_only_preloads_the_explicit_local_multilingual_checkpoint(self):
        seen = []

        def preload(router, names):
            seen.append((dict(router.models), names, router.max_loaded))
            return router

        with patch.dict('os.environ', {'LAYA_API_KEY': 'synthetic-test-key',
                                       'LAYA_MODEL_PATH': '/checkpoint/multilingual'}):
            with patch('laya.router.Router.preload', preload):
                module.create_app()
        self.assertEqual(seen, [({'multilingual': '/checkpoint/multilingual'}, ['multilingual'], 1)])

    def test_refuses_an_empty_server_key_before_model_loading(self):
        with patch.dict('os.environ', {'LAYA_API_KEY': ''}):
            with patch('laya.router.Router.preload') as preload:
                with self.assertRaises(RuntimeError):
                    module.create_app()
                preload.assert_not_called()

    def test_refuses_remote_checkpoint_identifiers_before_loading(self):
        with patch.dict('os.environ', {'LAYA_API_KEY': 'synthetic-test-key',
                                       'LAYA_MODEL_PATH': 'convaiinnovations/laya'}):
            with patch('laya.router.Router.preload') as preload:
                with self.assertRaises(RuntimeError):
                    module.create_app()
                preload.assert_not_called()


if __name__ == '__main__':
    unittest.main()
