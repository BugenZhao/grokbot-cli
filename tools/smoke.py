#!/usr/bin/env python3
"""End-to-end smoke test of the grokbot CLI against a real, dedicated test bot.

  GROKBOT_TEST_AGENT=<bot uuid or name> tools/smoke.py [--bin target/debug/grokbot] [--only ask,log]

Every step runs the real binary and checks exit codes and output. Replies are verified
with random tokens the bot is asked to echo, so a pass means the message really went
through and the reply really came back. Takes a few minutes and sends ~12 messages to
the test bot; never point it at a bot you care about. Requires a signed-in CLI
(`grokbot login` or `grokbot login --from-app`). Standard library only.

Each command, its exit code, stdout and stderr are printed as the test runs (truncated to
--max-lines per stream; the full transcript goes to --log). The assertions only check
mechanics; whoever runs this (usually a coding agent) is expected to read the outputs and
judge whether the bot's replies and the CLI's messages actually make sense. Email
addresses are redacted from the printed output and the log. `-q` prints results only.

Exit status: 0 if every step passed (skips allowed), 1 otherwise.
"""
import argparse, json, os, re, secrets, shlex, subprocess, sys, tempfile, time

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")


class Fail(Exception):
    pass


class Skip(Exception):
    pass


class Smoke:
    def __init__(self, binary, agent, log, quiet=False, max_lines=30):
        self.bin, self.agent = binary, agent
        self.env = {**os.environ, "GROKBOT_AGENT": agent}
        self.tokens = {}
        self.log, self.quiet, self.max_lines = log, quiet, max_lines

    def emit(self, text, full=None):
        """Print `text` (unless quiet) and write `full` (default: text) to the log."""
        if not self.quiet:
            print(text, flush=True)
        self.log.write((text if full is None else full) + "\n")
        self.log.flush()

    def _stream(self, name, data):
        lines = EMAIL_RE.sub("<email>", data).rstrip("\n").splitlines()
        if not lines:
            return
        shown = lines[: self.max_lines]
        extra = len(lines) - len(shown)
        fmt = lambda ls: "\n".join(f"    {name} │ {l}" for l in ls)
        self.emit(fmt(shown) + (f"\n    {name} │ … {extra} more lines (see log)" if extra else ""), fmt(lines))

    def run(self, *args, stdin=None, timeout=300, agent=True, show=True):
        """Run the CLI; with `show`, print the command line, exit code and both streams."""
        env = self.env if agent else {k: v for k, v in self.env.items() if k != "GROKBOT_AGENT"}
        t0 = time.time()
        try:
            p = subprocess.run([self.bin, *args], input=stdin, capture_output=True, text=True, timeout=timeout, env=env)
        except subprocess.TimeoutExpired:
            self.emit(f"  $ grokbot {shlex.join(args)}\n    (killed after {timeout}s)")
            raise
        if show:
            prefix = "" if agent else "env -u GROKBOT_AGENT "
            self.emit(f"  $ {prefix}grokbot {shlex.join(args)}" + (" < stdin" if stdin is not None else ""))
            if stdin is not None:
                self._stream("stdin ", stdin)
            self._stream("stdout", p.stdout)
            self._stream("stderr", p.stderr)
            self.emit(f"    → exit {p.returncode} in {time.time() - t0:.1f}s")
        return p.returncode, p.stdout, p.stderr

    def token(self, name):
        self.tokens[name] = f"smk-{secrets.token_hex(3)}"
        return self.tokens[name]

    def expect(self, cond, msg, out="", err=""):
        if not cond:
            # Outputs were already printed by run(); in quiet mode include a short tail.
            detail = "\n".join(s for s in (out.strip()[-300:], err.strip()[-300:]) if s) if self.quiet else ""
            raise Fail(EMAIL_RE.sub("<email>", msg + (f"\n      {detail}" if detail else "")))

    def echo_prompt(self, tok):
        return f"[cli smoke] Reply with exactly this token and nothing else: {tok}"

    # ---- steps (run in definition order) ----

    def step_whoami(self):
        """signed in; --json carries the account"""
        # Not shown: the output is all personal account data.
        code, out, err = self.run("whoami", "--json", show=False)
        self.expect(code == 0, f"exit {code}", "", err)
        me = json.loads(out)
        self.expect(me.get("me") is not None, "no account in --json output")
        self.emit(f"  $ grokbot whoami --json   (output hidden)\n    → exit 0, session mode {me.get('mode')!r}")

    def step_agents(self):
        """bot list includes the test bot"""
        code, out, err = self.run("agents", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        rows = [json.loads(l) for l in out.splitlines() if l.strip()]
        ids = {(r["agent"]["agentId"], r["agent"]["name"]) for r in rows}
        q = self.agent.lower()
        self.expect(any(i.startswith(q) or n.lower() == q for i, n in ids), "test bot not in `agents` output")

    def step_status(self):
        """status for the selected bot only"""
        code, out, err = self.run("status", "--json")
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(len(out.splitlines()) == 1, "expected exactly one status line for -a", out)

    def step_ask(self):
        """basic round trip; reply should be just the token"""
        tok = self.token("ask")
        code, out, err = self.run("ask", "--timeout", "3m", self.echo_prompt(tok))
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(tok in out, "reply does not contain the token", out, err)

    def step_ask_stdin_json(self):
        """message from stdin; --json event stream sent → reply → done"""
        tok = self.token("stdin")
        code, out, err = self.run("ask", "--json", "--timeout", "3m", stdin=self.echo_prompt(tok))
        self.expect(code == 0, f"exit {code}", out, err)
        events = [json.loads(l) for l in out.splitlines() if l.strip()]
        types = [e.get("type") for e in events]
        self.expect(types[:1] == ["sent"] and types[-1:] == ["done"], f"unexpected event sequence {types}", out)
        self.expect(any(e.get("type") == "reply" and tok in e.get("text", "") for e in events), "no reply event with the token", out)
        self.expect(events[-1].get("outcome") == "success", f"outcome {events[-1].get('outcome')!r}", out)

    def step_no_wait_then_wait(self):
        """--no-wait prints a message id; wait collects the reply"""
        tok = self.token("async")
        code, out, err = self.run("ask", "--no-wait", self.echo_prompt(tok))
        self.expect(code == 0, f"ask --no-wait exit {code}", out, err)
        mid = out.strip()
        self.expect(UUID_RE.match(mid) is not None, f"--no-wait did not print a message id: {mid!r}")
        code, out, err = self.run("wait", mid, "--timeout", "3m")
        self.expect(code == 0, f"wait exit {code}", out, err)
        self.expect(tok in out, "wait output does not contain the token", out, err)

    def step_timeout_then_resume(self):
        """--timeout exits 5 with a resume hint; wait then gets the full reply"""
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
        """-f uploads a file the bot can read"""
        tok = self.token("file")
        with tempfile.NamedTemporaryFile("w", suffix=".txt", prefix="smoke-", delete=False) as f:
            f.write(f"The secret word is {tok}\n")
        self.emit(f"  (attachment {os.path.basename(f.name)} contains: 'The secret word is {tok}')")
        try:
            code, out, err = self.run("ask", "-f", f.name, "--timeout", "3m",
                                      "[cli smoke] What is the secret word in the attached file? Reply with the word only.")
        finally:
            os.unlink(f.name)
        self.expect(code == 0, f"exit {code}", out, err)
        self.expect(tok in out, "reply does not contain the token from the attachment", out, err)

    def step_stop(self):
        """stop interrupts a running turn; its wait ends with outcome cancelled"""
        code, out, err = self.run("ask", "--no-wait",
                                  "[cli smoke] Use web search to look up today's weather in 3 different cities, one at a time, "
                                  "then summarise them in a table.")
        self.expect(code == 0, f"ask --no-wait exit {code}", out, err)
        mid = out.strip()
        deadline = time.time() + 30
        while time.time() < deadline:
            code, out, _ = self.run("status", "--json", show=False)
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
        """transcript is oldest-first and contains earlier replies"""
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
        """tasks / tasks --done / tasks --checklist / sessions succeed"""
        for args in (("tasks",), ("tasks", "--done"), ("tasks", "--checklist"), ("sessions",)):
            code, out, err = self.run(*args)
            self.expect(code == 0, f"`{' '.join(args)}` exit {code}", out, err)

    def step_routines(self):
        """toggle the test bot's first routine and restore it"""
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
        """unknown or missing bot exits 2 with a helpful message"""
        code, out, err = self.run("-a", "no-such-bot-" + secrets.token_hex(4), "status")
        self.expect(code == 2, f"unknown bot: expected exit 2, got {code}", out, err)
        code, out, err = self.run("ask", "hello", agent=False)
        self.expect(code == 2, f"no bot selected: expected exit 2, got {code}", out, err)

    def step_needs_input(self):
        """a widget reply makes ask exit 6 and say what the bot is waiting for"""
        # Last on purpose: it leaves an unanswered widget in the transcript.
        code, out, err = self.run("ask", "--timeout", "3m",
                                  "[cli smoke] Ask me 'A or B?' using a choice card with buttons (a widget), not plain text.")
        self.expect(code == 6, f"expected exit 6 (needs input), got {code}", out, err)
        self.expect("needs your input" in err, "missing needs-input message", out, err)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.environ.get("GROKBOT_BIN", "target/debug/grokbot"))
    ap.add_argument("--only", help="comma-separated step names (without the step_ prefix)")
    ap.add_argument("-q", "--quiet", action="store_true", help="print step results only")
    ap.add_argument("--max-lines", type=int, default=30, help="lines shown per output stream")
    ap.add_argument("--log", default=f".cache/smoke/{time.strftime('%Y%m%d-%H%M%S')}.log",
                    help="full transcript of every command (default: .cache/smoke/<timestamp>.log)")
    args = ap.parse_args()

    agent = os.environ.get("GROKBOT_TEST_AGENT", "").strip()
    if not agent:
        sys.exit("GROKBOT_TEST_AGENT is not set: point it at a dedicated throwaway bot (see AGENTS.md)")
    if not os.path.exists(args.bin):
        sys.exit(f"{args.bin} not found; build first (`just build`)")

    os.makedirs(os.path.dirname(args.log) or ".", exist_ok=True)
    log = open(args.log, "w")
    smoke = Smoke(args.bin, agent, log, quiet=args.quiet, max_lines=args.max_lines)
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
        doc = (getattr(Smoke, name).__doc__ or "").strip()
        if args.quiet:
            print(f"· {label} …", end=" ", flush=True)
        smoke.emit(f"\n━━ {label}" + (f": {doc}" if doc else ""))
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
        line = f"{status} ({time.time() - t0:.0f}s)" + (f"\n    {note}" if note else "")
        if args.quiet:
            print(line)
        smoke.emit(f"  ⇒ {label}: {line}")
        results.append((label, status))

    failed = [l for l, s in results if s == "FAIL"]
    summary = (f"\n{len(results) - len(failed)}/{len(results)} ok"
               + (f", {sum(s == 'skip' for _, s in results)} skipped" if any(s == "skip" for _, s in results) else "")
               + (f"; failed: {', '.join(failed)}" if failed else ""))
    print(summary)
    log.write(summary + "\n")
    log.close()
    print(f"full transcript: {args.log}")
    if not args.quiet:
        print("Mechanical checks done. Now read the outputs above: do the replies answer the prompts,"
              " and do the CLI's messages make sense? Report anything odd even if every step passed.")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
