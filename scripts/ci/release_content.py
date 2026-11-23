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


# The payload a supported package must carry, per artifact class, at the path it installs to. A
# name alone is not enough: the folder, its Node, its entry and its licences have to be at the
# place the app looks, in each artifact that installs them.
AGENT_PATHS = {
    "linux": ("usr/lib/intcad/mastra-agent", "lib/intcad/mastra-agent"),
    "macos": ("Contents/Resources/mastra-agent", "Resources/mastra-agent"),
    "windows": ("mastra-agent",),
}

# The macOS slices a universal app must carry a Node for.
SLICES = ("aarch64-apple-darwin", "x86_64-apple-darwin")


# The headless CLI archives are not the app and never carry the agent: the app package beside them
# does, and a command-line build has no Assistant in it. A standalone executable covered by an
# inspected package is judged by that package (its bytes are a subset of it).
def carries_no_agent(name):
    lowered = str(name).lower()
    return "-cli-" in lowered and lowered.endswith((".zip", ".tar.gz", ".tar.xz"))


def agent_problem(carried):
    """Why one artifact's agent does not work, or ``""`` when it does."""
    if not isinstance(carried, dict) or not carried.get("present"):
        return "carries no mastra-agent/ folder"
    if not carried.get("node"):
        return "the mastra-agent/ folder has no Node program"
    if carried.get("missing"):
        return "the agent is incomplete: missing " + _sample(carried.get("missing"))
    # INT-1473's work starts from the skills, and the modelling set is the one the release
    # promises; one unrelated skill is not it.
    skills = carried.get("skills") or []
    if not any(str(skill).startswith("modelling-skills/") for skill in skills):
        return "the agent carries no modelling skill (mastra-agent/skills/modelling-skills/<skill>/SKILL.md)"
    return ""


def _slices_for(identity):
    return SLICES if identity == "release-macos" else ()


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
            # Every artifact of a supported package must carry the agent whole, from its own
            # bytes: a correct zip beside a broken MSI is a failed package, not a passing one.
            per_artifact = report.get("agents")
            inspected = {
                item.get("name")
                for item in report.get("checked") or []
                if isinstance(item, dict)
            }
            if not isinstance(per_artifact, dict) or not per_artifact:
                failures.append(identity + ": no per-artifact agent evidence; the built package's "
                                "contents were not recorded one by one")
            else:
                # Every inspected artifact needs its own answer; an artifact the recorder looked
                # at but did not judge is a hole, not a pass.
                for name in sorted(inspected - set(per_artifact)):
                    failures.append(identity + ": " + str(name) + ": inspected without an agent "
                                    "verdict")
                for name in sorted(per_artifact):
                    if carries_no_agent(name) or (per_artifact[name] or {}).get("coveredBy"):
                        continue
                    carried = per_artifact[name]
                    problem = agent_problem(carried)
                    if problem:
                        failures.append(identity + ": " + name + ": " + problem)
                # At least one of the package's own artifacts must carry it: a set that holds
                # only the headless CLI archive has nothing a user can start the agent from.
                app_artifacts = [
                    name for name in per_artifact
                    if not carries_no_agent(name) and not (per_artifact[name] or {}).get("coveredBy")
                ]
                if not any((per_artifact[name] or {}).get("present") for name in app_artifacts):
                    failures.append(identity + ": no artifact of the package carries the agent")
            # A universal app's folder exists per CPU: both halves must be there, in the summary
            # and in every artifact's own answer.
            needed_slices = _slices_for(identity)
            if needed_slices:
                # The app's own artifacts only: a CLI archive and a covered executable carry no
                # slices of their own.
                summaries = [agent] if isinstance(agent, dict) else []
                for name in sorted(per_artifact or {}):
                    if carries_no_agent(name) or (per_artifact[name] or {}).get("coveredBy"):
                        continue
                    summaries.append(per_artifact[name])
                for carried in summaries:
                    if not isinstance(carried, dict):
                        continue
                    got = set(carried.get("nodeTargets") or [])
                    absent = [slice_ for slice_ in needed_slices if slice_ not in got]
                    if absent:
                        failures.append(identity + ": the package's agent has no Node for "
                                        + _sample(absent) + " (found " + _sample(sorted(got)) + ")")
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
        build = descriptors.get(identity) or {}
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
        # The repack is what a person installs: when it is a `mastra-agent` package, its own bytes
        # must carry the agent too, in every artifact of it. An architecture the agent is not
        # built for is native on both sides and is not asked for one.
        if report.get("distribution") == "mastra-agent":
            per_artifact = report.get("agents")
            own = {
                name: carried for name, carried in (per_artifact or {}).items()
                if name not in (report.get("sourceArtifact"),)
            }
            if not own:
                failures.append(identity + ": no evidence from the repack's own bytes; only its "
                                "source was looked at")
            inspected = {
                item.get("name")
                for item in report.get("checked") or []
                if isinstance(item, dict)
            }
            for name in sorted(inspected - set(per_artifact or {})):
                failures.append(identity + ": " + str(name) + ": inspected without an agent verdict")
            for name in sorted(own):
                if carries_no_agent(name) or (own[name] or {}).get("coveredBy"):
                    continue
                problem = agent_problem(own[name])
                if problem:
                    failures.append(identity + ": " + name + ": " + problem)
        # A repack is not a place to add a runtime the source did not carry.
        markers = [item.get("path") if isinstance(item, dict) else item
                   for item in report.get("markers") or []]
        foreign = [item.get("path") if isinstance(item, dict) else item
                   for item in report.get("agentMarkers") or []]
        if markers:
            failures.append(identity + ": carries a runtime marker: " + _sample(markers))
        if foreign:
            failures.append(identity + ": carries a Node or enterprise content outside the agent: "
                            + _sample(foreign))
        # The repack's bytes are checked like any other package's: every artifact inspected,
        # none left unread, and every digest the one this job built.
        unchecked = report.get("unchecked")
        if not isinstance(unchecked, list):
            failures.append(identity + ": unchecked artifact list is missing")
            continue
        if unchecked:
            names = [item.get("name") if isinstance(item, dict) else item for item in unchecked]
            failures.append(identity + ": content not inspected for " + _sample(names)
                            + "; the derived package is unproven")
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
        if checked != expected:
            missing = sorted(set(expected) - set(checked))
            extra = sorted(set(checked) - set(expected))
            changed = sorted(name for name in set(expected) & set(checked) if expected[name] != checked[name])
            failures.append(identity + ": derived content evidence does not cover the built package bytes"
                            + (f"; missing {_sample(missing)}" if missing else "")
                            + (f"; unexpected {_sample(extra)}" if extra else "")
                            + (f"; digest mismatch {_sample(changed)}" if changed else ""))
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
