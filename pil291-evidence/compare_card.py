import json
import os
import sys
import glob

chat_path, project = sys.argv[1], sys.argv[2]
chat = json.load(open(chat_path))
items = chat.get("items") or []
cards = [i for i in items if i.get("kind") == "dfm.check" and i.get("output")]
print(f"dfm.check items: {len(cards)}")
if not cards:
    sys.exit(1)
card = json.loads(cards[-1]["output"])
runs = sorted(glob.glob(os.path.join(project, ".intcad/checks/runs/*/")), key=os.path.getmtime, reverse=True)
if not runs:
    print("no run on disk")
    sys.exit(1)
run = runs[0]
report = json.load(open(os.path.join(run, "report.json")))
findings = json.load(open(os.path.join(run, "findings.json")))

print("card.run_id        =", card.get("runId"))
print("disk.runId         =", report.get("runId"))
print("card.model_version =", card.get("modelVersion"))
print("disk.modelVersion  =", report.get("modelVersion"))
print("card.candidate     =", card.get("candidateDigest"))
print("disk.candidate     =", report.get("candidateDigest"))
print("card.config        =", card.get("configDigest"))
print("disk.config        =", report.get("configDigest"))
print("card.status        =", card.get("status"), "outcome=", card.get("outcome"))
print("disk.state         =", report.get("state"))
print("card.findings      =", len(card.get("findings") or []))
print("disk.findings      =", len(findings.get("findings") or []))
print("card.source        =", json.dumps(card.get("source"), ensure_ascii=False)[:160])
print("disk.source        =", json.dumps(findings.get("source"), ensure_ascii=False)[:160])

by_id = {f.get("id"): f for f in findings.get("findings") or []}
same = 0
for c in (card.get("findings") or [])[:5]:
    d = by_id.get(c.get("id"))
    ok = d is not None and all(c.get(k) == d.get(k) for k in ("measured", "requirement", "status", "state", "rules"))
    same += 1 if ok else 0
    print(f"  finding {c.get('id')}: card(measured={c.get('measured')}, requirement={c.get('requirement')}, status={c.get('status')}, state={c.get('state')})"
          f" disk(measured={(d or {}).get('measured')}, requirement={(d or {}).get('requirement')}, status={(d or {}).get('status')}, state={(d or {}).get('state')}) match={ok}")
print(f"compared {same}/5 first findings equal on measured/requirement/status/state/rules")
