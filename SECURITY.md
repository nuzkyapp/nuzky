# Security

Nuzky opens project files and media that can come from anyone, so a bug there can reach someone's computer. Please report it privately.

## Report a vulnerability

Use [Report a vulnerability](https://github.com/nuzkyapp/nuzky/security/advisories/new) on the repository's Security tab. Do not open a public issue or pull request for it.

Include what an attacker controls (a project file, a media file, a model download), what they can do with it, and the steps or a file that shows it.

## What counts

- A `.nuzky` project or a media file that makes Nuzky read or write files outside the project, reach the network, run code, or crash.
- Anything that lets another program or user on the computer use the app's local IPC or the agent connection.
- The model downloads, which must match their pinned SHA-256, the daily update check, which reads only a version number, and the update the user installs with a click, which Nuzky must install only when its signature matches the key built into Nuzky.
- Sign in with ChatGPT: a token that leaves the system keychain for a file, a log, the app's web page or a server other than OpenAI's, or a sign-in that accepts an answer meant for another attempt or another account.
- The AI panel's ChatGPT: anything that lets it run something other than Nuzky's own tools, or sends its token anywhere but OpenAI.
- An agent's picture from the web (`import_web_image`): an address that makes Nuzky reach this computer, its network or a file on it, or fill its memory or disk.

Nuzky is a prototype and only the latest release gets fixes.
