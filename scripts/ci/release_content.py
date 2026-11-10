"""Decide whether a release is native-only from actual package content evidence.

`contents.json` is produced per package job by `package_contents.py` from the built
artifacts. #14/#15 (the Mastra installed-runtime requirements) are not applicable
only when the workflow declares a native distribution AND every canonical installer
was inspected and contains no runtime marker. A declaration, an env flag, a file
name or a hand-written boolean is never enough.
"""

DERIVED_PREFIXES = ("release-flatpak-", "release-arch-", "release-snap-")


def _artifacts(build):
    return {
        artifact.get("name"): artifact.get("sha256")
        for artifact in build.get("artifacts") or []
        if isinstance(artifact, dict) and isinstance(artifact.get("name"), str)
    }


def _sample(items, limit=5):
    shown = ", ".join(str(item) for item in items[:limit])
    more = "" if len(items) <= limit else f" (+{len(items) - limit} more)"
    return shown + more


def problems(source_revision, descriptors, contents):
    """Return every reason the native-content decision is not satisfied.

    ``descriptors`` maps packageId to a parsed build.json. ``contents`` maps
    packageId to a parsed contents.json. An empty list means the native decision
    is supported by the evidence; it is not a repository-wide success claim.
    """
    failures = []
    canonical = {
        identity: build
        for identity, build in descriptors.items()
        if not identity.startswith(DERIVED_PREFIXES)
    }
    native = {}
    for identity, build in sorted(canonical.items()):
        report = contents.get(identity)
        if not isinstance(report, dict):
            failures.append(identity + ": missing package content evidence (contents.json); "
                            "#15/#14 cannot be marked not applicable")
            continue
        if report.get("packageId") != identity:
            failures.append(identity + ": content evidence packageId mismatch")
        if report.get("sourceRevision") != source_revision:
            failures.append(identity + ": content evidence sourceRevision mismatch")
        distribution = report.get("distribution")
        if distribution != "native":
            failures.append(identity + ": distribution is " + str(distribution)
                            + ", not native; #15/#14 still block this distribution")
            continue
        markers = report.get("markers")
        if not isinstance(markers, list):
            failures.append(identity + ": content markers are missing")
            continue
        if markers:
            paths = [marker.get("path") if isinstance(marker, dict) else marker for marker in markers]
            failures.append(identity + ": declared native but the installed package contains the #15 runtime: "
                            + _sample(paths))
        unchecked = report.get("unchecked")
        if not isinstance(unchecked, list):
            failures.append(identity + ": unchecked artifact list is missing")
            continue
        if unchecked:
            names = [item.get("name") if isinstance(item, dict) else item for item in unchecked]
            failures.append(identity + ": content not inspected for " + _sample(names)
                            + "; native distribution is unproven")
        ignored = set(report.get("ignored") or [])
        checked = {
            item.get("name"): item.get("sha256")
            for item in report.get("checked") or []
            if isinstance(item, dict)
        }
        expected = {
            name: digest
            for name, digest in _artifacts(build).items()
            if name not in ignored
        }
        covered = {
            item.get("name"): item.get("coveredBy")
            for item in report.get("checked") or []
            if isinstance(item, dict) and item.get("coveredBy")
        }
        for name, covering in sorted(covered.items()):
            if covering not in checked or covering == name:
                failures.append(identity + ": " + str(name) + " claims coverage by a package that was not inspected: "
                                + str(covering))
        if checked != expected:
            missing = sorted(set(expected) - set(checked))
            extra = sorted(set(checked) - set(expected))
            changed = sorted(name for name in set(expected) & set(checked) if expected[name] != checked[name])
            failures.append(identity + ": content evidence does not cover the built package bytes"
                            + (f"; missing {_sample(missing)}" if missing else "")
                            + (f"; unexpected {_sample(extra)}" if extra else "")
                            + (f"; digest mismatch {_sample(changed)}" if changed else ""))
        native[identity] = distribution == "native" and not markers and not unchecked and checked == expected

    derived = {
        identity: build
        for identity, build in descriptors.items()
        if identity.startswith(DERIVED_PREFIXES)
    }
    for identity in sorted(derived):
        report = contents.get(identity)
        if not isinstance(report, dict):
            failures.append(identity + ": missing derived package content evidence (contents.json)")
            continue
        if report.get("packageId") != identity:
            failures.append(identity + ": derived content evidence packageId mismatch")
        if report.get("sourceRevision") != source_revision:
            failures.append(identity + ": derived content evidence sourceRevision mismatch")
        source = report.get("derivedFrom")
        if not native.get(source):
            failures.append(identity + ": derived package source is not a verified native package: " + str(source))
            continue
        source_artifacts = _artifacts(descriptors.get(source) or {})
        if source_artifacts.get(report.get("sourceArtifact")) != report.get("sourceSha256"):
            failures.append(identity + ": derived package source digest does not match the " + str(source)
                            + " package bytes")
    return failures
