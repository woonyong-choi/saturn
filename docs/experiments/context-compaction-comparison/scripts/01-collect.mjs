import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync, appendFileSync, existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const exp = new URL('../', import.meta.url);
const scenarios = readFileSync(new URL('../handoff-packet-quality-v2/data/raw/scenarios-20261001T185207Z-aa1e13a.jsonl', exp), 'utf8').trim().split('\n').map(JSON.parse);
const packets = new Map(readFileSync(new URL('../handoff-packet-quality-v2/data/raw/packets-20261001T185207Z-aa1e13a.jsonl', exp), 'utf8').trim().split('\n').map(line => { const x = JSON.parse(line); return [x.scenario_id, x]; }));
const source = process.env.FAST_JEV_SOURCE || '/tmp/saturn-compare-fast-jev';
const { compact, JevClient } = await import(pathToFileURL(`${source}/dist/index.js`));
const localDir = new URL('../../../../.local/experiments/context-compaction-comparison/', import.meta.url);
mkdirSync(localDir, { recursive: true });
const output = new URL('sessions-20261005T115947Z-145293d.jsonl', localDir);
const done = new Set(existsSync(output) ? readFileSync(output, 'utf8').trim().split('\n').filter(Boolean).map(x => JSON.parse(x).scenario_id) : []);
const limit = Number(process.argv[2] || 1);

function secretFromStdin() {
  return new Promise((resolve, reject) => {
    process.stdin.once('data', chunk => {
      const value = chunk.toString('utf8').trim();
      if (!value.startsWith('apikey_')) reject(new Error('API 키 형식이 맞지 않음'));
      else resolve(value);
      process.stdin.pause();
    });
    process.stdin.resume();
  });
}

function toMessages(record) {
  const messages = [];
  for (const item of record) {
    if (item.kind === 'tool') {
      const tool_use_id = `seq_${item.seq}`;
      messages.push({ role: 'assistant', text: '', toolUses: [{ tool_use_id, tool: item.tool, input: item.args }] });
      messages.push({ role: 'user', text: '', toolUses: [], toolResults: [{ tool_use_id, text: item.result }] });
    } else {
      messages.push({ role: item.kind === 'user' ? 'user' : 'assistant', text: `[seq ${item.seq}, session ${item.session}] ${item.text}`, toolUses: [] });
    }
  }
  return messages;
}

function render(messages) {
  return messages.map((m, i) => {
    const parts = [];
    if (m.text) parts.push(m.text);
    for (const t of m.toolUses) parts.push(`도구 호출 ${t.tool_use_id} ${t.tool} ${JSON.stringify(t.input)}`);
    for (const t of m.toolResults || []) parts.push(`도구 결과 ${t.tool_use_id}: ${t.text}`);
    return `${i + 1}. ${m.role}: ${parts.join('\n')}`;
  }).join('\n');
}

function claude(prompt) {
  return new Promise(resolve => {
    const started = Date.now();
    const child = spawn('claude', ['-p', '--model', 'sonnet', '--safe-mode', '--tools', '', '--no-session-persistence', '--output-format', 'json'], { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', x => stdout += x);
    child.stderr.on('data', x => stderr += x);
    child.on('close', code => resolve({ code, stdout, stderr: stderr.slice(0, 1000), elapsed_s: (Date.now() - started) / 1000 }));
    child.stdin.end(prompt);
  });
}

const answerRules = '아래 맥락만 근거로 질문에 답하라. 파일을 읽거나 명령을 실행하지 마라. 맥락에 근거가 없으면 unknown을 true로 둔다. 숫자는 숫자만, 날짜는 YYYY-MM-DD로 쓴다. 답은 JSON 배열 하나로만 쓴다: [{"id": "q1", "answer": "...", "unknown": false}]';
const key = await secretFromStdin();
const client = new JevClient({ apiKey: key, model: 'jev-1.13.0' });
let consecutiveFailures = 0;
for (const scenario of scenarios.filter(x => !done.has(x.scenario_id)).slice(0, limit)) {
  const messages = toMessages(scenario.record);
  const row = { scenario_id: scenario.scenario_id, ts_utc: new Date().toISOString(), source_commit: 'e3f262a7f4d42bd8dd32ced30d26176f7cb545b0', conditions: {} };
  try {
    let requests = 0, input_tokens = 0, output_tokens = 0;
    const asker = { ask: async (state, questions) => {
      for (let attempt = 0; ; attempt++) {
        try {
          const answer = await client.ask(state, questions);
          requests++;
          input_tokens += answer.usage?.input_tokens || 0;
          output_tokens += answer.usage?.output_tokens || 0;
          return answer;
        } catch (error) {
          if (attempt >= 2 || !/429|500|502|503|504|capacity|overload/i.test(String(error))) throw error;
          await new Promise(resolve => setTimeout(resolve, 1000 * (attempt + 1)));
        }
      }
    } };
    const result = await compact(messages, asker);
    row.compaction = { stats: result.stats, decisions: result.decisions, jev_usage: { requests, input_tokens, output_tokens } };
    const contexts = { 'saturn-packet': packets.get(scenario.scenario_id).packets['judge-all'].packet, 'fast-jev': render(result.messages) };
    const order = Number(scenario.scenario_id.slice(1)) % 2 ? ['fast-jev', 'saturn-packet'] : ['saturn-packet', 'fast-jev'];
    for (const condition of order) {
      const context = contexts[condition];
      const prompt = `${answerRules}\n\n# 맥락\n\n${context}\n\n# 질문\n\n${scenario.questions.map(q => `${q.qid}. ${q.text}`).join('\n')}`;
      const reply = await claude(prompt);
      row.conditions[condition] = { context_chars: context.length, context_sha256: createHash('sha256').update(context).digest('hex'), ...reply };
    }
    consecutiveFailures = 0;
  } catch (error) {
    row.error = String(error).replaceAll(key, '[redacted]').slice(0, 500);
    consecutiveFailures++;
  }
  appendFileSync(output, JSON.stringify(row) + '\n', { mode: 0o600 });
  process.stdout.write(`${scenario.scenario_id} ${row.error ? 'error' : 'ok'} ${JSON.stringify(row.compaction?.stats || {})}\n`);
  if (consecutiveFailures >= 5) break;
}
