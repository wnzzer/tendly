# Third-party brand vectors

Claude asterisk and OpenAI knot paths are sourced from `@lobehub/icons-static-svg@1.95.0` ([LobeHub Icons](https://github.com/lobehub/lobe-icons)). Codex is identified with the OpenAI mark **and the Codex text label**; this does not claim a distinct official Codex logo or any endorsement. Antigravity's mark is from `@lobehub/icons-static-svg@1.106.0`. Pi's mark is the shape of the logo pi.dev publishes (`https://pi.dev/logo-auto.svg`), redrawn in one colour at icon size. Brand trademarks remain their owners' property.

MIT License

Copyright (c) 2023 LobeHub

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


# Symbols Nerd Font Mono

`apps/web/src/assets/fonts/symbols-nerd-font-mono.woff2` is `SymbolsNerdFontMono-Regular.ttf` from [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts) v3.5.1 (`NerdFontsSymbolsOnly`), converted to WOFF2 without other changes. It is used only as a fallback font for shell-prompt icons in the terminal pane.

The patcher and font are MIT licensed (below). The glyphs come from the icon sets listed here, each under its own license:

| Icon set name          | upstream                                              | version         | license     |
|------------------------|-------------------------------------------------------|-----------------|-------------|
| Codicons               | https://github.com/microsoft/vscode-codicons          | 0.0.45          | CC BY 4.0   |
| Devicons               | https://github.com/devicons/devicon                   | 2.17.0          | MIT         |
| extraglyphs            | https://github.com/source-foundry/Hack                | -               | MIT         |
| Font Awesome           | https://github.com/FortAwesome/Font-Awesome           | 6.5.1           | CC BY 4.0   |
| Font Awesome Extension | https://github.com/AndreLZGava/font-awesome-extension | 0.0.3           | MIT         |
| Font Logos             | https://github.com/lukas-w/font-logos                 | 1.3.0           | unlicensed  |
| MaterialDesign         | https://github.com/Templarian/MaterialDesign-Font     | Oct 6, 2022     | Apache 2.0  |
| Octicons               | https://github.com/primer/octicons                    | 18.3.0          | MIT         |
| Seti and original      | https://github.com/jesseweed/seti-ui                  | 0.8.1           | MIT         |
| Pomicons               | https://github.com/gabrielelana/pomicons              | 1.001           | OFL 1.1 RFN |
| Powerline Extra        | https://github.com/ryanoasis/powerline-extra-symbols  | 1.200           | MIT         |
| Powerline Symbols      | https://github.com/powerline/powerline                | 1.000 (ca 2013) | MIT         |
| Power Symbols IEC      | https://github.com/jloughry/Unicode                   | Feb 2015        | MIT         |
| Weather Icons          | https://github.com/erikflowers/weather-icons          | 2.0.10 (1.100)  | OFL 1.1     |


The MIT License (MIT)

Copyright (c) 2014 Ryan L McIntyre

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


# Terminal libraries

These are npm dependencies of the web client, bundled into its build; each package ships its own license text.

| Package | Upstream | License | Use |
|---|---|---|---|
| `@wterm/dom`, `@wterm/ghostty` 0.5.4 | https://github.com/vercel-labs/wterm | Apache-2.0 | Default terminal renderer and its VT core. `@wterm/dom` carries a local patch (`patches/@wterm__dom@0.5.4.patch`) that reuses one DOM Range in the renderer. |
| Ghostty (`libghostty`, compiled to WebAssembly inside `@wterm/ghostty`) | https://github.com/ghostty-org/ghostty | MIT | Terminal emulation core used by wterm. |
| `@xterm/xterm` 6.0.0, `@xterm/addon-fit`, `@xterm/addon-webgl` | https://github.com/xtermjs/xterm.js | MIT | Fallback terminal renderer (`?terminal=xterm`). |
