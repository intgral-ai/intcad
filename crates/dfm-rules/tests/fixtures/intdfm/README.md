# INTdfm baseline projections

These files are compact projections of the INTdfm recordings in
`INTdfm/tests/fixtures/dfm-replay/v1/` at commit
`2f24830fcd84c8e3047eb6643139b94c27b8c1b6`. They hold only what the Rust rule tests need:
the declared values, the two profiles, the `SolidFacts` JSON and the expected findings for the
three hole rules. No STEP bytes and no absolute paths are here.

They are committed so the Rust crate builds and tests without a second checkout. Do not edit
them by hand; regenerate them from the INTdfm recordings with the same projection:

```sh
python - <<'PY'
import json
from pathlib import Path
src = Path("INTdfm/tests/fixtures/dfm-replay/v1")
out = Path("crates/dfm-rules/tests/fixtures/intdfm")
for name in ["hole_min_diameter.above", "hole_min_diameter.below",
             "hole_min_diameter.boundary", "hole_min_diameter.factory.below",
             "hole_to_edge.below", "hole_to_bend.below",
             "hole_to_bend.not_applicable", "missing_thickness",
             "holes_step_failed", "shop_not_applicable",
             "hole_to_edge.plate_holes", "hole_to_edge.opening",
             "hole_to_bend.u_holes"]:
    data = json.loads((src / f"{name}.json").read_text())
    declared = data["input"]["declared"]
    findings = [
        {key: finding[key] for key in ("rules", "status", "error_code", "reason_code",
                                       "measured", "requirement", "message")}
        for finding in data["expected"]["report"]["findings"]
        if any(rule in finding["rules"] for rule in
               ("sm.hole_min_diameter", "sm.hole_to_edge", "sm.hole_to_bend"))
    ]
    packed = {
        "source": {
            "intdfm_commit": data["source"]["intdfm_commit"],
            "core_source_sha256": data["source"]["core_source_sha256"],
            "report_version": data["source"]["report_version"],
        },
        "case": {
            "id": name,
            "kind": data["case"]["kind"],
            "rule": data["case"]["rule"],
            "description": data["case"]["description"],
            "part": data["case"]["part"],
        },
        "declared": {
            "process": declared.get("process"),
            "material": declared.get("material"),
            "thicknessMm": declared.get("thickness_mm"),
        },
        "generic": data["input"]["generic"],
        "shop": data["input"]["shop"],
        "facts": data["input"]["solid"],
        "comparison": data["comparison"],
        "expected": {"findings": findings},
    }
    (out / f"{name}.json").write_text(
        json.dumps(packed, sort_keys=True, indent=2, ensure_ascii=False) + "\n"
    )
PY
```

If the projection changes, update the INTdfm contract first and bump its schema version. A
changed `source.core_source_sha256` means the Python rule layer moved; re-record deliberately.
