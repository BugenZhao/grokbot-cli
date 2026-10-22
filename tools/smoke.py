#!/usr/bin/env python3
"""End-to-end smoke test of the grokbot CLI against a real, dedicated test bot.

  GROKBOT_TEST_AGENT=<bot uuid or name> tools/smoke.py [--bin target/debug/grokbot] [--only ask,log]

Every step runs the real binary and checks exit codes and output. Replies are verified
with random tokens the bot is asked to echo, so a pass means the message really went
through and the reply really came back. Takes a few minutes and sends ~12 messages to
the test bot; never point it at a bot you care about. Requires a signed-in CLI
(`grokbot login` or `grokbot login --from-app`). Standard library only.

Exit status: 0 if every step passed (skips allowed), 1 otherwise.
"""
import argparse, json, os, re, secrets, subprocess, sys, tempfile, time

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


class Fail(Exception):
    pass


class Skip(Exception):
    pass


class Smoke:
    def __init__(self, binary, agent):
        self.bin, self.agent = binary, agent
        self.env = {**os.environ, "GROKBOT_AGENT": agent}
        self.tokens = {}

    def run(self, *args, stdin=None, timeout=300, agent=True):
        env = self.env if agent else {k: v for k, v in self.env.items() if k != "GROKBOT_AGENT"}
        p = subprocess.run([self.bin, *args], input=stdin, capture_output=True, text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout, p.stderr

    def token(self, name):
        self.tokens[name] = f"smk-{secrets.token_hex(3)}"
        return self.tokens[name]

    @staticmethod
    def expect(cond, msg, out="", err=""):
        if not cond:
            detail = "\n".join(s for s in (out.strip()[-400:], err.strip()[-400:]) if s)
            raise Fail(msg + (f"\n      {detail}" if detail else ""))

    def echo_prompt(self, tok):
        return f"[cli smoke] Reply with exactly this token and nothing else: {tok}"

    # ---- steps (run in definition order) ----

    def step_whoami(self):
        code, out, err = self.run("whoami", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(json.loads(out).get("me") is not None, "no account in --json output", out)

    def step_agents(self):
        code, out, err = self.run("agents", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        rows = [json.loads(l) for l in out.splitlines() if l.strip()]
        ids = {(r["agent"]["agentId"], r["agent"]["name"]) for r in rows}
        q = self.agent.lower()
        self.expect(any(i.startswith(q) or n.lower() == q for i, n in ids), "test bot not in `agents` output")

    def step_status(self):
        code, out, err = self.run("status", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(len(out.splitlines()) == 1, "expected exactly one status line for -a", out)

    def step_ask(self):
        tok = self.token("ask")
        code, out, err = self.run("ask", "--timeout", "3m", self.echo_prompt(tok))
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(tok in out, "reply does not contain the token", out, err)

    def step_ask_stdin_json(self):
        tok = self.token("stdin")
        code, out, err = self.run("ask", "--json", "--timeout", "3m", stdin=self.echo_prompt(tok))
        self.expect(code == 0, f"exit {code}", out, err)
        events = [json.loads(l) for l in out.splitlines() if l.strip()]
        types = [e.get("type") for e in events]
        self.expect(types[:1] == ["sent"] and types[-1:] == ["done"], f"unexpected event sequence {types}", out)
        self.expect(any(e.get("type") == "reply" and tok in e.get("text", "") for e in events), "no reply event with the token", out)
        self.expect(events[-1].get("outcome") == "success", f"outcome {events[-1].get('outcome')!r}", out)

    def step_no_wait_then_wait(self):
        tok = self.token("async")
        code, out, err = self.run("ask", "--no-wait", self.echo_prompt(tok))
        self.expect(code == 0, f"ask --no-wait exit {code}", out, err)
        mid = out.strip()
        self.expect(UUID_RE.match(mid) is not None, f"--no-wait did not print a message id: {mid!r}")
        code, out, err = self.run("wait", mid, "--timeout", "3m")
        self.expect(code == 0, f"wait exit {code}", out, err)
        self.expect(tok in out, "wait output does not contain the token", out, err)

    def step_timeout_then_resume(self):
        tok = self.token("timeout")
        prompt = (f"[cli smoke] Write a short paragraph (about 100 words) about terminals, "
                  f"then put this token alone on the last line: {tok}")
        code, out, err = self.run("ask", "--timeout", "1s", prompt)
        self.expect(code == 5, f"expected exit 5 on timeout, got {code}", out, err)
        m = re.search(r"grokbot wait -a (\S+) (\S+)", err)
        self.expect(m is not None, "timeout message lacks a `grokbot wait` hint", out, err)
        code, out, err = self.run("wait", "-a", m.group(1), m.group(2), "--timeout", "3m")
        self.expect(code == 0, f"resume wait exit {code}", out, err)
        self.expect(tok in out, "resumed wait output does not contain the token", out, err)

    def step_attachment(self):
        tok = self.token("file")
        with tempfile.NamedTemporaryFile("w", suffix=".txt", prefix="smoke-", delete=False) as f:
            f.write(f"The secret word is {tok}\n")
        try:
            code, out, err = self.run("ask", "-f", f.name, "--timeout", "3m",
                                      "[cli smoke] What is the secret word in the attached file? Reply with the word only.")
        finally:
            os.unlink(f.name)
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(tok in out, "reply does not contain the token from the attachment", out, err)

    def step_stop(self):
        code, out, err = self.run("ask", "--no-wait",
                                  "[cli smoke] Use web search to look up today's weather in 3 different cities, one at a time, "
                                  "then summarise them in a table.")
        self.expect(code == 0, f"ask --no-wait exit {code}", out, err)
        mid = out.strip()
        deadline = time.time() + 30
        while time.time() < deadline:
            code, out, _ = self.run("status", "--json")
            if code == 0 and out.strip() and json.loads(out.splitlines()[0]).get("live", {}).get("isRunning"):
                break
            time.sleep(1)
        else:
            raise Fail("bot never reported running within 30s")
        code, out, err = self.run("stop")
        self.expect(code == 0, f"stop exit {code}", out, err)
        self.expect("Interrupted" in err, "stop did not report an interrupted run", out, err)
        code, out, err = self.run("wait", mid, "--timeout", "90s")
        self.expect(code == 1 and "cancelled" in err, f"expected wait to end with outcome cancelled (exit 1), got exit {code}", out, err)

    def step_log(self):
        code, out, err = self.run("log", "-n", "60", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        entries = [json.loads(l) for l in out.splitlines() if l.strip()]
        seqs = [e["seq"] for e in entries]
        self.expect(seqs == sorted(seqs), "log entries are not oldest-first")
        tok = self.tokens.get("ask")
        if tok:
            blob = json.dumps(entries, ensure_ascii=False)
            self.expect(tok in blob, "log does not contain the reply from step ask")
        code, out, err = self.run("log", "-n", "5")
        self.expect(code == 0 and out.strip(), f"plain log exit {code} / empty", out, err)

    def step_tasks(self):
        for args in (("tasks",), ("tasks", "--done"), ("tasks", "--checklist"), ("sessions",)):
            code, out, err = self.run(*args)
            self.expect(code == 0, f"`{' '.join(args)}` exit {code}", out, err)

    def step_routines(self):
        code, out, err = self.run("routines", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        rows = [json.loads(l) for l in out.splitlines() if l.strip()]
        if not rows:
            raise Skip("test bot has no routine; ask it to create a far-future one to cover toggling")
        r = rows[0]
        rid, was_on = r["automationId"], bool(r["record"].get("isEnabled"))
        first, second = ("disable", "enable") if was_on else ("enable", "disable")
        try:
            code, out, err = self.run("routines", first, rid[:8], "--json")
            self.expect(code == 0, f"routines {first} exit {code}", out, err)
            self.expect(json.loads(out)["record"]["isEnabled"] == (first == "enable"), f"{first} did not take effect", out)
        finally:
            code, out, err = self.run("routines", second, rid[:8], "--json")
        self.expect(code == 0 and json.loads(out)["record"]["isEnabled"] == was_on, "failed to restore the routine's state", out, err)

    def step_usage_errors(self):
        code, out, err = self.run("-a", "no-such-bot-" + secrets.token_hex(4), "status")
        self.expect(code == 2, f"unknown bot: expected exit 2, got {code}", out, err)
        code, out, err = self.run("ask", "hello", agent=False)
        self.expect(code == 2, f"no bot selected: expected exit 2, got {code}", out, err)

    def step_needs_input(self):
        # Last on purpose: it leaves an unanswered widget in the transcript.
        code, out, err = self.run("ask", "--timeout", "3m",
                                  "[cli smoke] Ask me 'A or B?' using a choice card with buttons (a widget), not plain text.")
        self.expect(code == 6, f"expected exit 6 (needs input), got {code}", out, err)
        self.expect("needs your input" in err, "missing needs-input message", out, err)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.environ.get("GROKBOT_BIN", "target/debug/grokbot"))
    ap.add_argument("--only", help="comma-separated step names (without the step_ prefix)")
    args = ap.parse_args()

    agent = os.environ.get("GROKBOT_TEST_AGENT", "").strip()
    if not agent:
        sys.exit("GROKBOT_TEST_AGENT is not set: point it at a dedicated throwaway bot (see AGENTS.md)")
    if not os.path.exists(args.bin):
        sys.exit(f"{args.bin} not found; build first (`just build`)")

    smoke = Smoke(args.bin, agent)
    steps = [n for n in Smoke.__dict__ if n.startswith("step_")]
    if args.only:
        wanted = {f"step_{s.strip()}" for s in args.only.split(",")}
        unknown = wanted - set(steps)
        if unknown:
            sys.exit(f"unknown steps: {', '.join(sorted(s[5:] for s in unknown))}; have: {', '.join(s[5:] for s in steps)}")
        steps = [s for s in steps if s in wanted]

    results = []
    for name in steps:
        label = name[5:]
        print(f"· {label} …", end=" ", flush=True)
        t0 = time.time()
        try:
            getattr(smoke, name)()
            status, note = "pass", ""
        except Skip as e:
            status, note = "skip", str(e)
        except Fail as e:
            status, note = "FAIL", str(e)
        except subprocess.TimeoutExpired:
            status, note = "FAIL", "command timed out"
        except Exception as e:  # malformed output etc.
            status, note = "FAIL", f"{type(e).__name__}: {e}"
        print(f"{status} ({time.time() - t0:.0f}s)" + (f"\n    {note}" if note else ""))
        results.append((label, status))

    failed = [l for l, s in results if s == "FAIL"]
    print(f"\n{len(results) - len(failed)}/{len(results)} ok"
          + (f", {sum(s == 'skip' for _, s in results)} skipped" if any(s == "skip" for _, s in results) else "")
          + (f"; failed: {', '.join(failed)}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
