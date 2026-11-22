"""Decide a release's distribution from actual package content evidence.

`contents.json` is produced per package job by `package_contents.py` from the built
artifacts. The declaration in the workflow is checked against what the packages
really carry:

- ``native``: every canonical installer was inspected and holds no runtime marker,
  so #14/#15 (the Mastra installed-runtime requirements) are not applicable;
- ``mastra-agent``: every canonical installer carries the built-in agent's folder
  whole (its own Node, entry, dependencies and licences) and no legacy runtime;
- ``mastra``: refused, whatever the evidence says. A declaration, an env flag, a
  file name or a hand-written boolean is never enough.
"""

DERIVED_PREFIXES = ("release-flatpak-", "release-arch-", "release-snap-")
DISTRIBUTIONS = ("native", "mastra", "mastra-agent")
# The canonical package identities that must carry the built-in agent when the release declares
# one. These are the supported targets the agent is built for; the other architectures stay
# native and say so.
AGENT_IDENTITIES = ("release-linux-x86_64", "release-macos", "release-windows-x64")


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


def problems(source_revision, descriptors, contents, declaration="native"):
    """Return every reason the declared distribution is not supported by the evidence.

    ``descriptors`` maps packageId to a parsed build.json. ``contents`` maps
    packageId to a parsed contents.json. An empty list means the declaration is
    supported by the package bytes; it is not a repository-wide success claim.
    """
    failures = []
    if declaration not in DISTRIBUTIONS:
        return ["the workflow declares an unknown distribution: " + str(declaration)]
    if declaration == "mastra":
        return ["#15/#14: the legacy Mastra workflow runtime is declared; "
                "it is not approved for a release and no package evidence can approve it"]
    carrying_agent = declaration == "mastra-agent"
    # A release that ships the agent ships it in every supported package; the architectures the
    # agent is not built for stay native and say so.
    def expected_distribution(identity):
        return "mastra-agent" if carrying_agent and identity in AGENT_IDENTITIES else "native"
    canonical = {
        identity: build
        for identity, build in descriptors.items()
        if not identity.startswith(DERIVED_PREFIXES)
    }
    proven = {}
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
        needed = expected_distribution(identity)
        if distribution != needed:
            failures.append(identity + ": content was recorded as " + str(distribution)
                            + ", the release needs " + needed + " for this package")
            continue
        agent = report.get("agent")
        if carrying_agent and identity in AGENT_IDENTITIES:
            # The canonical packages must carry the agent whole, from their own bytes.
            if not isinstance(agent, dict) or not agent.get("present"):
                failures.append(identity + ": declared mastra-agent but the built package carries no "
                                "mastra-agent/ folder")
            else:
                if not agent.get("node"):
                    failures.append(identity + ": the built package's mastra-agent/ folder has no Node program")
                if agent.get("missing"):
                    failures.append(identity + ": the built package's agent is incomplete: missing "
                                    + _sample(agent.get("missing")))
                # INT-1473's agentic work starts from the modelling skills: a package whose agent
                # carries none ships an agent that cannot do the job it is packaged for.
                if not agent.get("skills"):
                    failures.append(identity + ": the built package's agent carries no skills "
                                    "(mastra-agent/skills/<set>/<skill>/SKILL.md)")
        elif isinstance(agent, dict) and agent.get("present"):
            failures.append(identity + ": carries mastra-agent/ but the release supports the agent on "
                            + _sample(AGENT_IDENTITIES) + " only")
        markers = report.get("markers")
        if not isinstance(markers, list):
            failures.append(identity + ": content markers are missing")
            continue
        if markers:
            paths = [marker.get("path") if isinstance(marker, dict) else marker for marker in markers]
            failures.append(identity + ": the installed package contains the #15/#14 runtime: " + _sample(paths))
        if carrying_agent:
            foreign = report.get("agentMarkers")
            if not isinstance(foreign, list):
                failures.append(identity + ": agent markers are missing")
                continue
            if foreign:
                paths = [marker.get("path") if isinstance(marker, dict) else marker for marker in foreign]
                failures.append(identity + ": a Node program or enterprise content outside mastra-agent/: "
                                + _sample(paths))
        unchecked = report.get("unchecked")
        if not isinstance(unchecked, list):
            failures.append(identity + ": unchecked artifact list is missing")
            continue
        if unchecked:
            names = [item.get("name") if isinstance(item, dict) else item for item in unchecked]
            failures.append(identity + ": content not inspected for " + _sample(names)
                            + "; the distribution is unproven, so #15/#14 still apply")
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
        proven[identity] = distribution == needed and not markers and not unchecked and checked == expected

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
        # The derivative itself is not inspected; its report only carries what the
        # source scan found. A failed scan or a runtime marker there must fail too,
        # otherwise a Flatpak/Snap/Arch package could pass on an unproven source.
        markers = report.get("markers")
        if not isinstance(markers, list) or markers:
            failures.append(identity + ": derived package source has unproven runtime markers")
            continue
        unchecked = report.get("unchecked")
        if not isinstance(unchecked, list) or unchecked:
            failures.append(identity + ": derived package source was not inspected")
            continue
        source = report.get("derivedFrom")
        if not proven.get(source):
            failures.append(identity + ": derived package source is not a verified package: " + str(source))
            continue
        # A derived package is the source tarball repacked: it carries what the source carries.
        if report.get("distribution") != (contents.get(source) or {}).get("distribution"):
            failures.append(identity + ": derived package distribution " + str(report.get("distribution"))
                            + " does not match its source " + str(source))
        source_artifacts = _artifacts(descriptors.get(source) or {})
        source_digest = report.get("sourceSha256")
        # Arch copies a prerelease tarball under its pkgver name before packaging. The
        # bytes are the inspected Linux package even when the file name differs.
        if source_artifacts.get(report.get("sourceArtifact")) != source_digest and source_digest not in source_artifacts.values():
            failures.append(identity + ": derived package source digest does not match the " + str(source)
                            + " package bytes")
    return failures
