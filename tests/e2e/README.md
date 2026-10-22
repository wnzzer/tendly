# Browser suite

The app driven as a person uses it: a real gateway, the built page, a headless
Chrome. Each file starts its own gateway (`harness.mjs`) with a fresh state
directory, a free port and a small Git repository as its workspace. Claude
Code, Codex and Pi are the synthetic clients in
`packages/native-bridge/fixtures`, so no account, model or network is used.

```sh
pnpm test:e2e                                  # builds the page and server, then runs every *.e2e.mjs
node --test tests/e2e/canvas.e2e.mjs           # one file, against what is already built
TENDLY_E2E_SERVER=/path/to/tendly-server …     # a release binary, with its own page and bridge
TENDLY_E2E_WEB_DIR=/path/to/dist …             # another build of the page
TENDLY_E2E_CHROME=/path/to/chrome …            # another Chrome (default: /Applications/Google Chrome.app)
TENDLY_E2E_KEEP=1 …                            # keep the gateway's directory for a look afterwards
```

The browser runs headless and never takes a window. The gateway is stopped by
its own PID, so a gateway running on this machine for real is left alone.

A test seeds what it needs through the API (`seed`, `setLayout`), then works
only through the page. `browser.errors` collects uncaught page errors, console
errors and failed requests; most tests end by asserting it is empty.

The fixture clients answer by scenario: a message reading `approve`, `ask`,
`write`, `hold`, `fail`, `subagent`… makes Claude's stand-in behave that way
(see the `switch` in `claude-cli.mjs`); anything else is answered "Hello 🦊".
