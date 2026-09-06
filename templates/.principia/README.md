# .principia

This directory is how this repository contributes itself to the Principia
launchpad. It is committed on purpose: clone the repo anywhere and its
launchpad entry comes with it.

| File | What it is |
| --- | --- |
| `repo.json` | Flows, agents and tasks this repo offers. The only required file. |
| `agents/*.md` | Prompts this repo ships, runnable from the launchpad. |
| `local.json` | Machine-local values. Gitignored. Written by the extension, not by you. |

You are not meant to write these by hand. Ask an agent:

```
Set up this repo for the Principia launchpad. Follow spec/v1/SPEC.md.
```

Then check your work:

```sh
node scripts/validate.js .
```
