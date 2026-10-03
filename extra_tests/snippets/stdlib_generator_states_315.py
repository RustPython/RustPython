"""An async generator awaiting a value has a suspended frame but an active step."""

import unittest


class Pause:
    def __await__(self):
        yield "paused"


class StateTests(unittest.TestCase):
    def test_async_generator_await_and_yield(self):
        async def generate():
            await Pause()
            yield 1

        g = generate()
        step = g.__anext__()
        try:
            self.assertEqual(step.send(None), "paused")
            self.assertTrue(g.ag_running)
            self.assertEqual(g.ag_state, "AGEN_SUSPENDED")
            with self.assertRaises(StopIteration) as exc:
                step.send(None)
            self.assertEqual(exc.exception.value, 1)
        finally:
            step.close()
            with self.assertRaises(StopIteration):
                g.aclose().send(None)


if __name__ == "__main__":
    unittest.main()
