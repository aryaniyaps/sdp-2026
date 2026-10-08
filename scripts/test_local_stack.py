"""Offline startup must fail closed instead of fetching missing dependencies."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('local_stack', Path(__file__).with_name('local-stack.py'))
stack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stack)


class OfflineStartupTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.state = Path(self.tmp.name)
        (self.state / 'docker-compose.yml').touch()
        (self.state / 'access.code').write_text('test-password')
        self.config = {'services': {
            'engine': {'image': 'app:local', 'environment': {}},
            'caddy': {'image': 'caddy:2', 'ports': [{'target': 80, 'published': '18088'}]},
            'ollama-init': {'image': 'ollama:local'},
            'gpu-tunnel': {'image': 'tunnel:local', 'profiles': ['gpu-tunnel']},
        }}
        for target, value in [('state', self.state), ('dc', None), ('run', None), ('request', None)]:
            ctx = patch.object(stack, target, value) if value is not None else patch.object(stack, target)
            mock = ctx.start()
            self.addCleanup(ctx.stop)
            setattr(self, target, mock)
        self.dc.return_value = json.dumps(self.config)
        self.request.return_value = {'database': True, 'embedder': True, 'degraded_mode': False}

    def invoke(self, mode='up'):
        with patch('sys.argv', ['local-stack.py', mode]):
            stack.main()

    def test_start_excludes_download_jobs_and_forbids_build_and_pull(self):
        self.invoke()
        self.dc.assert_any_call('up', '-d', '--pull', 'never', '--no-build', 'engine', 'caddy')
        inspected = [c.args[3] for c in self.run.call_args_list if c.args[:3] == ('docker', 'image', 'inspect')]
        self.assertEqual(inspected, ['app:local', 'caddy:2'])

    def test_dashboard_api_is_checked_through_proxy(self):
        self.invoke('check')
        self.assertTrue(any(str(c.args[-1]).endswith('/api/v2/projects') for c in self.run.call_args_list))

    def test_missing_dashboard_route_fails_readiness(self):
        def response(*args, **kwargs):
            if str(args[-1]).endswith('/api/v2/projects'):
                raise subprocess.CalledProcessError(22, 'curl')
        self.run.side_effect = response
        with patch.object(stack.time, 'sleep'), self.assertRaisesRegex(RuntimeError, '/api/v2/projects'):
            self.invoke('check')

    def test_check_does_not_start_containers(self):
        self.invoke('check')
        self.assertFalse(any(c.args[0] == 'up' for c in self.dc.call_args_list))

    def test_missing_image_fails_before_start(self):
        self.run.side_effect = subprocess.CalledProcessError(1, 'docker')
        with self.assertRaisesRegex(RuntimeError, 'Missing local image'):
            self.invoke()
        self.assertFalse(any(c.args[0] == 'up' for c in self.dc.call_args_list))

    def test_missing_model_does_not_pull(self):
        self.request.side_effect = [self.request.return_value, subprocess.CalledProcessError(1, 'curl')]
        with self.assertRaisesRegex(RuntimeError, 'Cannot load installed model'):
            self.invoke()
        self.assertTrue(all('/api/pull' not in str(c) for c in self.request.call_args_list))


if __name__ == '__main__':
    unittest.main()
