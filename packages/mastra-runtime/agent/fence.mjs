// Which folders Mastra Code's file tools may reach (INT-1473: files only in the project folder).
//
// Mastra Code lets its file tools reach the session's folder (the tools' base, where a relative
// path lands), the folders a session adds (the project the app hands over), the skill folders of
// the agent's home, its plans folder, and two temporary folders as a scratch space: `os.tmpdir()`
// and `/tmp` (`DEFAULT_ALLOWED_PATHS`, `agents/workspace.js` of SDK 1.19.1). Two of these are taken
// off here:
// - `/tmp`, written into the list as it is: on Linux and macOS every program of the computer
//   shares it. (The entry points `os.tmpdir()` at a folder of the agent's own, main.mjs.)
// - The session's own folder. Mastra Code reads `AGENTS.md` and `CLAUDE.md` there on every turn
//   and puts them in the model's instructions, so a file the model wrote there, by a relative
//   path, would rewrite its own instructions for the rest of the conversation.
// What the tools are told about where they may go is made to say the same.
//
// The session's folder is empty, and to a read it is: a file read there is not found, not
// refused. `find_files` and `search_content` read the base's `.gitignore` before anything else and
// take only "not found" for "no such file"; refused, they failed on every call.

import { realpath } from "node:fs/promises";
import { resolve, sep } from "node:path";

/**
 * Makes Mastra's `LocalFilesystem` leave its base folder and the folders `denied` out of the
 * folders it lets a path be in. Its two checks are replaced: the one every operation makes
 * (`_isWithinAnyRoot`) and the one a read makes again after opening the file (`containedRoots`).
 * Each does what Mastra's does, over the folders that are left. So is the sentence that tells the
 * model where its file tools reach (`_getDefaultInstructions`). A file read in the base folder
 * (`readFile`) fails as a missing file. If the class is not the one this was written for, the
 * agent would run with those folders open: it throws instead, and the agent does not start.
 */
export function keepFilesFrom(Filesystem, denied) {
  const proto = Filesystem?.prototype;
  const needed = ["_isWithinAnyRoot", "containedRoots", "_resolvePathForContainment", "_isWithinRoot", "_getDefaultInstructions", "readFile"];
  if (!proto || needed.some((name) => typeof proto[name] !== "function")) {
    throw new Error("Mastra's file tools are not the ones this agent was written for: they would reach /tmp and the session's folder");
  }
  const off = denied.map((path) => resolve(path));
  // Mastra Code also lists skill folders inside the session's folder (`.agents/skills` and the
  // like): they are the session's folder too.
  const inside = (path, folder) => path === folder || path.startsWith(folder.endsWith(sep) ? folder : `${folder}${sep}`);
  const allowed = (fs) => {
    const base = typeof fs._basePath === "string" ? resolve(fs._basePath) : null;
    return [...new Set((fs._allowedPaths ?? []).filter((path) => typeof path === "string").map((path) => resolve(path)))].filter((path) => !(base && inside(path, base)) && !off.includes(path));
  };
  proto._isWithinAnyRoot = function _isWithinAnyRoot(absolutePath) {
    const resolvedPath = this._resolvePathForContainment(absolutePath);
    if (!resolvedPath) return false;
    return allowed(this).some((root) => {
      const resolvedRoot = this._resolvePathForContainment(root);
      return resolvedRoot ? this._isWithinRoot(resolvedPath, resolvedRoot) : false;
    });
  };
  proto.containedRoots = async function containedRoots() {
    const roots = [];
    for (const root of allowed(this)) {
      try {
        roots.push(await realpath(root));
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
    }
    return roots;
  };
  const readFile = proto.readFile;
  proto.readFile = async function readFileOutsideTheBase(inputPath, options) {
    const base = typeof this._basePath === "string" ? resolve(this._basePath) : null;
    if (base && typeof inputPath === "string" && inside(resolve(base, inputPath), base)) {
      throw Object.assign(new Error(`ENOENT: no such file or directory, open '${inputPath}'`), { code: "ENOENT", path: inputPath });
    }
    return readFile.call(this, inputPath, options);
  };
  proto._getDefaultInstructions = function _getDefaultInstructions() {
    const roots = allowed(this);
    return roots.length > 0
      ? `File tools reach only these folders, by absolute paths: ${roots.join(", ")}.`
      : "File tools reach no folder in this session.";
  };
}

/** The folders taken off besides the session's own: `/tmp`, where it is not the agent's own temporary folder. */
export function deniedFolders(tmp) {
  return [resolve("/tmp")].filter((path) => path !== resolve(tmp));
}
