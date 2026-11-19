# Tempo end-to-end tests

The real app on a private, headless Wayland compositor (mutter), driven by real input.
Run `scripts/e2e.sh`; read [docs/TESTING_E2E.md](../../docs/TESTING_E2E.md) for what it
guarantees, how to write a scenario (`scenarios/NN_name.py`), the API of `tempo_e2e.py`,
and how to read the report.

Quick forms: `scripts/e2e.sh 02_transitions` (by name), `scripts/e2e.sh /any/path/check.py`
(a scenario file anywhere), `E2E_NO_BUILD=1 scripts/e2e.sh` (reuse the build). The runner
alone, from anywhere: `python3 tests/e2e/tempo_e2e.py run --report /tmp/out <scenario ...>`
(it uses the binary and fixture the script left; `TEMPO_E2E_APP` and `TEMPO_E2E_PROJECT`
pick others). The app is started
with `TEMPO_NO_AUDIO=1` and `TEMPO_NO_PROXY=1`.
