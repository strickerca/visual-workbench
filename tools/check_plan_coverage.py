#!/usr/bin/env python3
"""Check that IMPLEMENTATION_PLAN.md and prompts/ stay consistent with REQUIREMENTS.json.

Checks:
  1. Every requirement ID in REQUIREMENTS.json is assigned to at least one task in the plan
     (a "**Requirements:**" line or the Requirements column of a phase table).
  2. Every requirement-shaped ID mentioned anywhere in the plan or prompts exists.
  3. Every v1 requirement ID (from an optional v1 file) still exists.
  4. Every prompt file names a task that exists in the plan.
  5. The last task (in plan order) that lists a requirement belongs to the requirement's own phase,
     so the requirement can pass at its own gate.
  6. Each prompt's requirement IDs and dependencies match its task in the plan.
  7. Each gate task depends, directly or transitively, on the last listing task of every requirement
     its gate's scenarios (and re-runs) check.

Usage: python tools/check_plan_coverage.py [--v1 path/to/v1/REQUIREMENTS.json]
Exit code 0 = consistent, 1 = problems found.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REQ_PATH = ROOT / "docs" / "REQUIREMENTS.json"
PLAN_PATH = ROOT / "docs" / "IMPLEMENTATION_PLAN.md"
PROMPTS = ROOT / "prompts"

ID_RE = re.compile(r"\b(?:CORE|PEN|EDIT|SEL|CAPTURE|TUNNEL|CONNECT|DISPLAY|ADOBE|FORMAT|EXPORT|UX|QUALITY|"
                   r"INSTR|SEM|PKG|AGENT|AIEDIT|VERIFY|GHOST|SYNC|STREAM|EDITOR|LIC|SEC|DEV|PERF|HOOK)-\d{3}\b")
TASK_RE = re.compile(r"\bT\d\.\d{2}[ab]?\b")
HEADING_RE = re.compile(r"^### (T\d\.\d{2}[ab]?) ")
ROW_RE = re.compile(r"^\| (T\d\.\d{2}[ab]?) ")


def task_phase(task: str) -> int:
    return int(task[1])


def expand_tasks(text: str, order: list) -> set:
    """Task IDs named in a dependency phrase, expanding 'Tx.yy–Tx.zz' / 'through' ranges and 'every Phase N task'."""
    found = set(TASK_RE.findall(text))
    for a, b in re.findall(r"(T\d\.\d{2}[ab]?)\s*(?:–|-|through|to)\s*(T\d\.\d{2}[ab]?)", text):
        if a in order and b in order:
            i, j = order.index(a), order.index(b)
            found.update(order[i:j + 1])
    for ph in re.findall(r"every Phase (\d) task", text):
        found.update(t for t in order if task_phase(t) == int(ph))
    return found


def main() -> int:
    problems = []
    reqs = json.loads(REQ_PATH.read_text(encoding="utf-8"))["requirements"]
    req_ids = {r["id"] for r in reqs}
    req_phase = {r["id"]: r["phase"] for r in reqs}
    plan = PLAN_PATH.read_text(encoding="utf-8")

    # Parse the plan: task order, requirement lists and dependency phrases.
    order, listed, deps = [], {}, {}
    current = None
    for line in plan.splitlines():
        m = HEADING_RE.match(line)
        if m:
            current = m.group(1)
            order.append(current)
            listed.setdefault(current, [])
            continue
        if current and "**Depends on:**" in line:
            seg = line.split("**Depends on:**", 1)[1].split("·")[0]
            deps[current] = seg
        if current and "**Requirements:**" in line:
            seg = line.split("**Requirements:**", 1)[1].split("**Informs:**")[0]
            listed[current] += ID_RE.findall(seg)
        m = ROW_RE.match(line)
        if m:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            # Phase task tables only: Task | Size | Depends on | Requirements | Deliverables and acceptance
            if len(cells) != 5 or cells[1] not in ("S", "M", "L"):
                continue
            current = None
            task = m.group(1)
            if task in order:
                problems.append(f"Task {task} appears twice in the plan")
                continue
            order.append(task)
            listed[task] = ID_RE.findall(cells[3])
            deps[task] = cells[2]

    # 1. Assignment coverage.
    assigned = {i for ids in listed.values() for i in ids}
    unassigned = sorted(req_ids - assigned)
    if unassigned:
        problems.append(f"Requirements not assigned to any task ({len(unassigned)}): {', '.join(unassigned)}")

    # 2. Unknown IDs mentioned in the plan or prompts.
    texts = {"IMPLEMENTATION_PLAN.md": plan}
    prompt_files = sorted(PROMPTS.rglob("T*.md")) if PROMPTS.exists() else []
    for p in prompt_files:
        texts[str(p.relative_to(ROOT))] = p.read_text(encoding="utf-8")
    for name, text in texts.items():
        unknown = sorted(set(ID_RE.findall(text)) - req_ids)
        if unknown:
            problems.append(f"{name} mentions unknown requirement IDs: {', '.join(unknown)}")

    # 3. v1 preservation.
    if "--v1" in sys.argv:
        v1_path = pathlib.Path(sys.argv[sys.argv.index("--v1") + 1])
        v1_ids = {r["id"] for r in json.loads(v1_path.read_text(encoding="utf-8"))["requirements"]}
        missing = sorted(v1_ids - req_ids)
        if missing:
            problems.append(f"v1 IDs missing from v2: {', '.join(missing)}")

    # 5. The last listing task is in the requirement's own phase.
    for rid in sorted(req_ids):
        tasks = [t for t in order if rid in listed.get(t, [])]
        if tasks and task_phase(tasks[-1]) != req_phase[rid]:
            problems.append(f"{rid} (phase {req_phase[rid]}) is last listed by {tasks[-1]} in another phase; "
                            "it could never pass at its own gate")

    # 7. Each gate task depends (directly or transitively) on the last listing task of every requirement
    #    its scenarios check at that gate (scenarios gated there plus re-runs).
    graph = {t: expand_tasks(deps.get(t, ""), order) - {t} for t in order}

    def ancestors(t, seen=None):
        seen = set() if seen is None else seen
        for d in graph.get(t, ()):
            if d not in seen:
                seen.add(d)
                ancestors(d, seen)
        return seen

    gate_task = {1: "T1.12", 2: "T2.11", 3: "T3.12", 4: "T4.09", 5: "T5.07"}
    scenarios = json.loads(REQ_PATH.read_text(encoding="utf-8")).get("scenarios", [])
    last_listing = {}
    for rid in req_ids:
        tasks = [t for t in order if rid in listed.get(t, [])]
        if tasks:
            last_listing[rid] = tasks[-1]
    for g, gt in gate_task.items():
        if gt not in order:
            problems.append(f"Gate task {gt} for G{g} is missing from the plan")
            continue
        reach = ancestors(gt) | {gt}
        for sc in scenarios:
            if sc.get("gate") != f"G{g}" and f"G{g}" not in sc.get("rerun_at", []):
                continue
            for rid in sc["requirements"]:
                if req_phase[rid] <= g and rid in last_listing and last_listing[rid] not in reach:
                    problems.append(f"{gt} (gate G{g}) checks {sc['id']} but does not depend on "
                                    f"{last_listing[rid]}, the last task listing {rid}")

    # 4 and 6. Prompts match plan tasks.
    for p in prompt_files:
        task = p.name.split("-")[0]
        if task not in order:
            problems.append(f"Prompt {p.name} names task {task}, which is not in the plan")
            continue
        text = texts[str(p.relative_to(ROOT))]
        m = re.search(r"\*\*Requirement IDs this task covers:\*\* (.*)", text)
        prompt_ids = sorted(set(ID_RE.findall(m.group(1)))) if m else []
        if prompt_ids != sorted(set(listed.get(task, []))):
            problems.append(f"Prompt {p.name} covers {prompt_ids}, plan lists {sorted(set(listed.get(task, [])))}")
        m = re.search(r"\*\*Depends on:\*\* (.*?)(?: Each dependency|$)", text, re.M)
        if m and task in deps:
            pd = expand_tasks(m.group(1), order)
            plan_d = expand_tasks(deps[task], order)
            if pd != plan_d:
                problems.append(f"Prompt {p.name} depends on {sorted(pd)}, plan says {sorted(plan_d)}")

    if problems:
        print("PLAN COVERAGE: PROBLEMS FOUND")
        for msg in problems:
            print(" -", msg)
        return 1
    print(f"PLAN COVERAGE: OK ({len(req_ids)} requirements assigned; {len(order)} tasks; "
          f"{len(prompt_files)} prompt files checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
