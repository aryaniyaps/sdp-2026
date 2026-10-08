"""Run the existing distillation stages through an authenticated, tool-free Pi.

No credentials are copied to the dataset. Requests contain synthetic sessions
only, are cached, and record usage/model provenance. Provider/model must be
available through this user's Pi installation. This adapter keeps Azure teacher
configuration and the serving model independent.

python pi_teacher.py scenarios generate --count 32 --workers 4
python pi_teacher.py label --workers 4
"""
from __future__ import annotations

import hashlib
import importlib
import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

import teacher


class PiTeacher:
    def __init__(self, cache_dir, usage_log=None, effort='low', attempts=2):
        self.provider = os.environ.get('PI_TEACHER_PROVIDER', 'openai')
        self.model = os.environ.get('PI_TEACHER_MODEL', 'gpt-5.6-sol')
        self.cache_dir = Path(cache_dir)/'pi'
        self.usage_log = usage_log
        self.effort = effort
        self.calls = self.cached = self.input_tokens = self.output_tokens = 0
        self._lock = threading.Lock()

    def complete(self, system, user, *, effort=None, max_output_tokens=24000, json_mode=True, tag=''):
        body = {'provider': self.provider, 'model': self.model, 'system': system,
                'user': user, 'effort': effort or self.effort, 'max_output_tokens': max_output_tokens}
        digest = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
        path = self.cache_dir/digest[:2]/f'{digest}.json'
        if path.exists():
            with self._lock:
                self.cached += 1
            return json.loads(path.read_text())['text']
        instructions = system + '\nReturn one JSON object only. No markdown fences. No tools.\n'
        with tempfile.TemporaryDirectory(prefix='sdp-teacher-') as folder:
            cmd = ['pi', '--provider', self.provider, '--model', self.model,
                   '--no-extensions', '--no-skills', '--no-prompt-templates', '--no-context-files',
                   '--no-builtin-tools', '--no-session', '--thinking', body['effort'],
                   '--system-prompt', instructions, '--mode', 'json', user]
            result = subprocess.run(cmd, cwd=folder, capture_output=True, text=True, timeout=300)
        if result.returncode:
            raise teacher.TeacherError(f'Pi exited {result.returncode}: {result.stderr[-500:]}')
        messages = []
        for line in result.stdout.splitlines():
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get('type') == 'message_end' and record.get('message', {}).get('role') == 'assistant':
                messages.append(record['message'])
        if not messages:
            raise teacher.TeacherError('Pi returned no assistant message')
        message = messages[-1]
        if message.get('stopReason') in ('error', 'aborted'):
            raise teacher.TeacherError(message.get('errorMessage', 'Pi request failed'))
        text = ''.join(c.get('text', '') for c in message.get('content', []) if c.get('type') == 'text').strip()
        if text.startswith('```'):
            text = text.split('\n', 1)[1].rsplit('```', 1)[0].strip()
        if not text:
            raise teacher.TeacherError('Pi returned empty text')
        usage = message.get('usage', {})
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(f'.{os.getpid()}.{threading.get_ident()}.tmp')
        tmp.write_text(json.dumps({'text': text, 'usage': usage, 'tag': tag,
                                  'provider': self.provider, 'model': self.model}))
        tmp.replace(path)
        with self._lock:
            self.calls += 1
            self.input_tokens += usage.get('input', 0)
            self.output_tokens += usage.get('output', 0)
            if self.usage_log:
                with Path(self.usage_log).open('a') as f:
                    f.write(json.dumps({'tag': tag, 'provider': self.provider, 'model': self.model, 'usage': usage})+'\n')
        return text

    def complete_json(self, system, user, **kwargs):
        text = self.complete(system, user, **kwargs)
        try:
            return json.loads(text)
        except json.JSONDecodeError as exc:
            raise teacher.TeacherError(f'teacher reply is not JSON: {exc}: {text[:200]!r}') from exc

    def summary(self):
        return f'Pi teacher {self.provider}/{self.model}: {self.calls} calls, {self.cached} cached, {self.input_tokens} input, {self.output_tokens} output tokens'


def configure_stage(stage):
    if stage.__name__ == 'scenarios' and os.environ.get('PI_TEACHER_COMPACT') == '1':
        original = stage.build_prompt
        def compact(seed):
            text = original(seed)
            text = text.replace('A session has 16 to 34 events', 'A session has 10 to 14 events')
            text = text.replace('Real tool output is long: at least a third of the tool results must be 800 to 2500 characters, two or three per session 2500 to 4500 characters (a full test run, a stack trace, a config file, a log), the rest short.', 'Include one realistic tool result of 800 to 1200 characters per session; keep the others under 300 characters.')
            return text + '\nThis local corpus represents compact retained turns. Keep the complete reply under 18000 characters while preserving all planted facts and the required corrections and traps.'
        stage.build_prompt = compact


def main():
    allowed = {'scenarios', 'label', 'label_turns', 'aux_tasks', 'evaluate'}
    if len(sys.argv) < 2 or sys.argv[1] not in allowed:
        sys.exit('usage: pi_teacher.py <scenarios|label|label_turns|aux_tasks|evaluate> [stage arguments]')
    module = sys.argv.pop(1)
    teacher.Teacher = PiTeacher
    stage = importlib.import_module(module)
    configure_stage(stage)
    stage.main()


if __name__ == '__main__':
    main()
