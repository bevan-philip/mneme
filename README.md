# mneme, a CLI for Anki
A vibe-coded CLI for Anki. This README isn't LLM generated, though.

## Why
- I wanted to make it easy enough for agents to interact with Anki. 
- While plugins introducing [MCP servers](https://ankiweb.net/shared/info/124672614) exist, I'd prefer to not add anything to Anki if I can help it. It adds something to keep up-to-date and synchronise with Anki version releases.
- [apy](https://github.com/lervag/apy/) exists, and as far as I can tell, it does most of what anyone would do.
- However, my agent (just so you know *how* vibe-coded this is) wanted [this functionality](https://github.com/bevan-philip/anki-cli/pull/7), so, here we are.

## Core ideas 
- We're leveraging Anki's [rslib](https://github.com/ankitects/anki/tree/main/rslib), so all behaviour should be equivalent to Anki.
- Anki will not be able to run simultaneously.
