# Third-party material

What in this repository came from somewhere else, under what terms, and how
much of it. Three kinds, and the distinction matters for what a reader may do
with each:

- **Vendored bytes**: files copied in verbatim. Their licence travels with them.
- **Quoted prose**: sentences quoted with attribution inside comments and
  documents, to show where a claim came from.
- **Facts**: protocol observations -- a byte value, a payload layout -- that this
  code relies on and that another project published. Facts are cited, not
  copied; no code from these projects is in this repository.

## mos9527/SonyHeadphonesClient (MIT) -- vendored bytes, and the reference

`tests/fixtures/mdr/*.bin` are 70 frames captured from a Sony WF-1000XM5 on
firmware 6.1.0, vendored unmodified from
<https://github.com/mos9527/SonyHeadphonesClient> at commit
`db0ae2574d8f8e1407e8ea28a7ee87db9dde1c24`, path `tests/WF-1000XM5-6.1.0/`.
`tests/fixtures/mdr/PROVENANCE.md` says why they must stay byte-identical.

The same commit's `libmdr` headers (`ProtocolV2.hpp`, `ProtocolV2T1.hpp`,
`ProtocolV2T2.hpp`) are the byte-level reference the protocol code was ported
from, and its `libmdr/src/HeadphonesV2*.cpp` are cited as the reference
client's behaviour. Citations of the form `T1:1234` are line numbers in
`ProtocolV2T1.hpp` at that commit, `T2:` in `ProtocolV2T2.hpp`, and `V2:` in
`libmdr/src/HeadphonesV2.cpp`. No C++ is vendored.

    MIT License

    Copyright (c) 2026 mos9527, Amr Satrio and other contributors

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

## delarosa1312/omarchy-sony-xm5 (MIT) -- quoted prose and one excerpt

Sentences from its `docs/protocol.md` and `docs/hardening.md` are quoted, with
attribution, in comments and in `tests/fixtures/mdr/PROVENANCE.md`, which also
quotes four lines of its `daemon/lib/mdr.py`. No other code was taken.

    MIT License

    Copyright (c) 2026 Delarosa1312

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

## Facts cited from other projects (no material copied)

These projects publish observations about real Sony headsets that this code
relies on or tests against. Each citation names its source where it is used.
Their licences are listed so a reader knows why nothing of theirs is copied
here: a fact about a byte is not their code, and their code is not ours to take.

| Project | Licence | What is cited |
|---|---|---|
| [Freeyourgadget/Gadgetbridge](https://codeberg.org/Freeyourgadget/Gadgetbridge) | AGPL-3.0-or-later | `SonyProtocolImplV2`'s payload layouts and its WH-1000XM5 coordinator's capabilities; two 10-byte NOTIFY payloads from `SonyProtocolImplV2Test`, used as test input in `tests/sony_session.rs` |
| [snizovtsev/xm5-mode-switch](https://github.com/snizovtsev/xm5-mode-switch) | GPL-2.0-or-later | A WH-1000XM5 observed answering NC/ASM at inquired type 0x17 |
| [PrivateiFox/XMDeck](https://github.com/PrivateiFox/XMDeck) | not stated | Pull request #5: NC/ASM at type 0x19 on WH-1000XM5/XM6 |
| [mos9527/SonyHeadphonesClient pull request #64](https://github.com/mos9527/SonyHeadphonesClient/pull/64) | MIT | The connection-quality write at inquired type 0x00, and multipoint changes needing an ALERT confirmation (tested on a WF-LC900, not an XM5) |
| [VyomJain6904/sony-headphones-linux](https://github.com/VyomJain6904/sony-headphones-linux) | see the project | The state document's paths, shape and command language, which `qs-sonyd` keeps compatible with |
