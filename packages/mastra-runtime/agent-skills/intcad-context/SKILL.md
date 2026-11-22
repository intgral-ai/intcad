---
name: intcad-context
description: Use when your instructions carry a project brief from Intcad's right rail, when the user asks what the project holds or requires, or before any change to a part that takes more than one step.
---

# Working from the project's context

The project brief in your instructions is the right rail as Intcad shows it: the files of the project folder, and where the latest DFM report is. It is your context library. Start every task from it; the user does not need to point you at files.

## Every task

1. **Find the context.** Read the brief's file list and run `inspect.dfm` for the latest DFM report. Open the files the task depends on (requirements, design intent, drawings, the shop's notes) and leave the rest closed. Run `project.inspect` when the task depends on the project's settings or its critical dimensions. Done when you can name each file the task depends on, or have found that the project has none.
2. **Plan in words, before the first change.** For a task of more than one step, your first message is the plan: a short numbered list of what changes, in which order, and how each step will be checked. Then work through it a step at a time, saying which step you are on.
3. **Check your own work.** After a change to the model, measure it with the Intcad tools and go through the requirements one by one: each with its source, the value measured and whether it passes. On a failure, fix it and check again, at most three rounds; then report what still fails and why. For manufacturability, read the DFM report and ask the user to run **Run DFM Check**; its new report is what `inspect.dfm` then returns. Say when the report you read was made: one from before your change says nothing about the change.
4. **Ask when unclear.** When a requirement is missing, ambiguous or contradicts another, ask in a plain message and wait for the answer. A conflict between two requirements is the user's to settle: name both, say which you would favour and why, and leave both as they are until the user answers. When a step would touch something a file marks as fixed (a critical dimension, a face or a value it says must stay), stop at that step and ask first.

Answer in the language the user writes in.

## Files and numbers

- Cite each requirement you rely on with its file: "requirements.md: wall at least 2 mm".
- Take every number from a tool: `measure`, `inspect_design`, the DFM report, or the text of a file. Where none gives it, write "not measured".
- Before changing a file, show the change: the file, the lines, the new text. Keep what is there; remove text only when the user asked for it.
- Open a file before saying what it holds or whether it can be read: the file tools read PDFs and images too. When a tool gives back nothing you can use, say so by name and ask the user for what you need from it.
- Give the file tools absolute paths that start with the project folder the brief names.
- A file added after the session opened is not in the brief: `find_files` finds it.
- File tools are for the project's files. The design is built, changed, measured and checked through the Intcad tools.
- Save, export or overwrite the design when the user asks for it; until then, say the change is ready to save.
