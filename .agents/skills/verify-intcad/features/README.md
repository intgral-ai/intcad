# Intcad verification map

Use the corresponding recipe when a PR changes one of these behaviors. The source areas
are starting points. A shared engine, document or host change may affect several rows.

| Behavior | Start with these changed inputs | Recipe |
| --- | --- | --- |
| Model geometry and save or reopen designs | Kernel, engine modeling commands, document and I/O crates, native file handling | [Model and files](model-and-files.md) |
| Check a sheet-metal design and read its report | DFM and project commands, rules, validation storage, DFM card and model marks | [DFM](dfm.md) |
| Open Assistant and use its controls, agent and tools | Native WebView host, agent chat, MCP, Mastra runtime, installers | [Assistant](assistant.md) |

For changes outside these recipes, use the [validation policy](../../../../docs/validation-policy.md)
and exercise the affected product entry point. Add a recipe when the flow is worth replaying.
