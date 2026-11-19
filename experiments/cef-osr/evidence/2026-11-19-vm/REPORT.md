# IntCAD E2E acceptance — linux

- Run: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740`
- Source root: `/home/hyjing/Code/intgral/intcad/worktrees/cef-osr`
- Commit: `219916a85709946de16f942f02ef2420ccb95617`; dirty files: 7
- Platform: linux; task: `cef-theme`; VM: `intcad-e2e-cef-osr-76ade07ca5d0-cef-theme`
- Package: direct Linux binary (debug sections stripped)
- Artifact: `/home/hyjing/.local/state/intcad-e2e/builds/cef-osr-76ade07ca5d0/219916a85709-c26aa9b5ea0e`
- Artifact SHA-256: `f25bb0acfb586eab4aa0f1fe77842f4a9199d7859c280d54935a698fd66694a8`
- Guest executable: `/home/intcad/intcad-e2e/run/solidcraft`
- Started: 2026-11-19T01:52:23Z

## Steps

- **1. build the selected source — PASS**
  - artifact f25bb0acfb586eab4aa0f1fe77842f4a9199d7859c280d54935a698fd66694a8; 219916a85709946de16f942f02ef2420ccb95617
  - evidence: `/home/hyjing/.local/state/intcad-e2e/builds/cef-osr-76ade07ca5d0/219916a85709-c26aa9b5ea0e/artifact.json`
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/logs/build.log`
- **2. deploy and start from the expected path — PASS**
  - guest /home/intcad/intcad-e2e/run/solidcraft sha256=f25bb0acfb586eab4aa0f1fe77842f4a9199d7859c280d54935a698fd66694a8; expected f25bb0acfb586eab4aa0f1fe77842f4a9199d7859c280d54935a698fd66694a8
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/screenshots/01-started.png`
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/logs/run.log`
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/logs/app.log`
- **3. open sample and see model — PASS**
  - sample=True; clicked (576,387); hover={'body': 'Plate', 'index': 16, 'mid': [0.0, 25.0, 8.0], 'point': [0.0, 26.454409655744534, 8.0], 'type': 'edge'}
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/screenshots/10-model.png`
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/logs/documents.json`
- **4. real drag rotates the model — PASS**
  - camera before={"distance": 127.10017644681146, "fov": 0.0, "pitch": 0.6154797086703874, "roll": 0.0, "target": [28.51827060951336, 14.81248548554642, 12.29421487603306], "up" after={"distance": 127.10017644681146, "fov": 0.0, "pitch": 0.29555783367038735, "roll": 0.0, "target": [28.51827060951336, 14.81248548554642, 12.29421487603306], "up
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/screenshots/11-rotated.png`
- **5. click a face selects it — PASS**
  - clicked at (648,405); selection=[{"body": "Plate", "index": 13, "point": [36.1336988799481, 15.178415790171186, 22.00000000000003], "type": "face"}]
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/screenshots/12-face-selected.png`
  - evidence: `/home/hyjing/.local/state/intcad-e2e/runs/intcad-e2e-cef-osr-76ade07ca5d0-cef-theme/20261119T015109Z-1152740/logs/selection.json`
- **6. open Assistant and a control responds — BLOCKED**
  - Assistant input/control failed: Assistant not open/page: {"agent": "", "agents": [], "askHere": {"open": false, "text": ""}, "attachments": [], "authMethods": [], "authStatus": null, "configOptions": null, "configPending": false, "connectAtStart": false, "currentAgent": null, "customModels": false, "detection": {"found": [], "missing": [{"basic": true, "i
- **7. resize and Assistant stays in place — BLOCKED**
  - step 6 failed; not executed
- **8. panel control after resize responds — BLOCKED**
  - step 6 failed; not executed

## Overall: **FAIL**

PASS only when all required steps are PASS.
