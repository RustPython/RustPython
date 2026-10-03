"""Generator state attributes introduced for the Python 3.15 inspect API."""

import unittest


class Pause:
    def __await__(self):
        yield "paused"


class StateTests(unittest.TestCase):
    def test_generator_lifecycle(self):
        def generate():
            self.assertEqual(g.gi_state, "GEN_RUNNING")
            yield 1

        g = generate()
        self.assertEqual(g.gi_state, "GEN_CREATED")
        self.assertEqual(next(g), 1)
        self.assertEqual(g.gi_state, "GEN_SUSPENDED")
        with self.assertRaises(StopIteration):
            next(g)
        self.assertEqual(g.gi_state, "GEN_CLOSED")
        with self.assertRaises(AttributeError):
            g.gi_state = "GEN_CREATED"

    def test_coroutine_lifecycle(self):
        async def run():
            self.assertEqual(c.cr_state, "CORO_RUNNING")
            await Pause()

        c = run()
        self.assertEqual(c.cr_state, "CORO_CREATED")
        self.assertEqual(c.send(None), "paused")
        self.assertEqual(c.cr_state, "CORO_SUSPENDED")
        with self.assertRaises(StopIteration):
            c.send(None)
        self.assertEqual(c.cr_state, "CORO_CLOSED")
        with self.assertRaises(AttributeError):
            c.cr_state = "CORO_CREATED"

    def test_async_generator_await_and_yield(self):
        async def generate():
            self.assertEqual(g.ag_state, "AGEN_RUNNING")
            await Pause()
            yield 1

        g = generate()
        self.assertEqual(g.ag_state, "AGEN_CREATED")
        step = g.__anext__()
        self.assertEqual(step.send(None), "paused")
        self.assertTrue(g.ag_running)
        self.assertEqual(g.ag_state, "AGEN_SUSPENDED")
        with self.assertRaises(StopIteration) as exc:
            step.send(None)
        self.assertEqual(exc.exception.value, 1)
        self.assertFalse(g.ag_running)
        self.assertEqual(g.ag_state, "AGEN_SUSPENDED")
        close = g.aclose()
        with self.assertRaises(StopIteration):
            close.send(None)
        self.assertEqual(g.ag_state, "AGEN_CLOSED")
        with self.assertRaises(AttributeError):
            g.ag_state = "AGEN_CREATED"

    def test_close_before_start(self):
        def generate():
            yield 1

        async def run():
            pass

        async def agenerate():
            yield 1

        g = generate()
        g.close()
        self.assertEqual(g.gi_state, "GEN_CLOSED")
        c = run()
        c.close()
        self.assertEqual(c.cr_state, "CORO_CLOSED")
        ag = agenerate()
        with self.assertRaises(StopIteration):
            ag.aclose().send(None)
        self.assertEqual(ag.ag_state, "AGEN_CLOSED")


if __name__ == "__main__":
    unittest.main()
