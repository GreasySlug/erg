"""Run every .er through both backends and classify the transpiler's failures.

Works on a scratch copy of tests/should_ok and examples so no .py/.pyc litter
lands in the worktree. Output: summary.tsv (one row per file) + details/<name>.txt

    python3 tests/transpile_survey.py <worktree> <outdir> [python]
"""
import os, shutil, subprocess, sys, re
from pathlib import Path

MARKER = "<<<erg_transpile_survey>>>"
WT = Path(sys.argv[1]); OUT = Path(sys.argv[2]); ERG = WT / "target/debug/erg"
# The generated .py has to run under the interpreter `erg` compiled the bytecode
# with, or the two backends are not being compared. Defaults to the one running
# this script (inside a venv that is the venv's, which is what erg picks too).
PY = sys.argv[3] if len(sys.argv) > 3 else sys.executable
OUT.mkdir(parents=True, exist_ok=True); (OUT / "details").mkdir(exist_ok=True)
SCRATCH = OUT / "src"
if SCRATCH.exists(): shutil.rmtree(SCRATCH)
SCRATCH.mkdir()
for d in ("tests/should_ok", "examples"):
    shutil.copytree(WT / d, SCRATCH / d, ignore=shutil.ignore_patterns("*.pyc", "__pycache__"))
    for er in (SCRATCH / d).glob("*.er"):
        if er.name.endswith(".d.er"): continue
        er.write_text(f'print! "{MARKER}"\n' + er.read_text())

def after_marker(out):
    return out.split(MARKER, 1)[1].lstrip("\n") if MARKER in out else out

def run(args, cwd):
    try:
        p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -999, "", "TIMEOUT"

def py_error_kind(stderr):
    lines = [l for l in stderr.strip().splitlines() if l.strip()]
    if not lines: return "unknown"
    last = lines[-1]
    m = re.match(r"^(\w+(?:Error|Exception|Warning|Interrupt|Exit))(?::\s*(.*))?", last)
    if m: return f"{m.group(1)}: {(m.group(2) or '')[:80]}"
    return last[:100]

def transpile_error_kind(out):
    text = out.strip()
    # erg's error output: "Error[#NNNN]: ..." followed by message lines; also panics
    if "panicked at" in text:
        m = re.search(r"panicked at ([^\n]*)\n([^\n]*)", text)
        return "PANIC " + ((m.group(1) + " :: " + m.group(2)) if m else text[:100])
    m = re.search(r"Error\[#(\d+)\][^\n]*\n(?:.*\n)*?([^\n]*(?:not (?:yet )?(?:implemented|supported)|unsupported|cannot|unimplemented)[^\n]*)", text, re.I)
    if m: return f"E{m.group(1)} {m.group(2).strip()[:100]}"
    m = re.search(r"Error\[#(\d+)\]", text)
    if m:
        # take the first non-empty line after the code line
        after = text[m.end():].split("\n")
        msg = next((l.strip() for l in after[1:] if l.strip() and not l.strip().startswith(("|", "-", "^"))), "")
        return f"E{m.group(1)} {re.sub(chr(27)+r'\[[0-9;]*m', '', msg)[:100]}"
    return text.splitlines()[-1][:100] if text else "unknown"

rows = []
files = sorted(list((SCRATCH / "tests/should_ok").glob("*.er")) + list((SCRATCH / "examples").glob("*.er")))
for f in files:
    name = f"{f.parent.name}/{f.stem}"
    rel = str(f.relative_to(SCRATCH))
    bc_rc, bc_out, bc_err = run([str(ERG), rel], SCRATCH)
    if bc_rc != 0:
        cls, info = "BC_FAIL", (bc_err.strip().splitlines() or ["?"])[-1][:100]
        rows.append((name, cls, info)); continue
    tr_rc, tr_out, tr_err = run([str(ERG), "--mode", "transpile", rel], SCRATCH)
    py = f.with_suffix(".py")
    if tr_rc != 0 or not py.exists():
        cls, info = "TRANSPILE_FAIL", transpile_error_kind(tr_out + tr_err)
        (OUT / "details" / (name.replace("/", "_") + ".txt")).write_text(tr_out + tr_err)
        rows.append((name, cls, info)); continue
    py_rc, py_out, py_err = run([PY, str(py.relative_to(SCRATCH))], SCRATCH)
    if py_rc != 0:
        cls, info = "PY_ERROR", py_error_kind(py_err)
        (OUT / "details" / (name.replace("/", "_") + ".txt")).write_text(py_out + "\n--- stderr ---\n" + py_err + "\n--- py ---\n" + py.read_text())
    elif after_marker(py_out) != after_marker(bc_out):
        cls, info = "MISMATCH", f"bc={after_marker(bc_out).strip()[:40]!r} py={after_marker(py_out).strip()[:40]!r}"
        (OUT / "details" / (name.replace("/", "_") + ".txt")).write_text("--- bytecode ---\n" + bc_out + "\n--- transpiled ---\n" + py_out + "\n--- py ---\n" + py.read_text())
    else:
        cls, info = "OK", ""
    rows.append((name, cls, info))
    print(f"{cls:15} {name}", flush=True)

with open(OUT / "summary.tsv", "w") as fh:
    for r in rows: fh.write("\t".join(r) + "\n")
from collections import Counter
print("\n=== totals ===")
for k, v in Counter(r[1] for r in rows).most_common(): print(f"{v:4} {k}")
