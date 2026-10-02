# Capturing a WH-1000XM5

One session with the headset on a desk settles most of `docs/OUTSTANDING.md`.
`tools/acceptance.sh` runs it and waits where a person is needed; this is what
it does and why each step is where it is. Nothing here has been run: there is
no adapter where it was written, and the script has only run as `--dry-run`.

## What it needs

- A WH-1000XM5, paired and connected, and nothing else talking to it: Sony's
  app on a phone holds the headset's one control channel.
- BlueZ, with `btmon` (bluez-utils) and root to run it.
- This package installed, or a checkout's `target/release` on `PATH`.
- `cargo`, for the one test that needs a Bluetooth stack.
- jayos, for the screenshot only.

## The session

**1. Trace first.** `btmon -w <dir>/trace.btsnoop` as root, before the daemon
dials, so the trace holds the L2CAP signalling that names the RFCOMM channel
and the whole handshake. A trace started later still imports -- every dynamic
channel is tried -- but it starts mid-conversation.

**2. The daemon, recording, with alerts on.** Stop the shell's daemon
(`sony-daemon.sh off` in jayos), then run `qs-sonyd run --alerts
--record=<dir>/record`. The recording is both directions of the daemon's own
link, named as the corpus is, so it settles even if `btmon` missed something.
`--alerts` makes the headset ask before a disruptive change, which is how
step 4 is confirmed. The directory is created private: it will hold the name
and address of every device the headset knows.

**3. Every verb once** with `qs-sony send`: the noise modes, an ambient level,
focus on voice, both connection priorities, pause and play, DSEE, a volume.
The replies to these are items 2 to 5.

**4. A general-setting write**, `general multipoint off`. With alerts on, the
headset asks first; `qs-sony status` shows it under `alert`, and `qs-sony send
alert yes` answers. A yes to a disconnecting message is followed by the
headset dropping the link, which the daemon records as a clean close.

**5. Reconnect, then read it back.** Disconnect and connect the headset
(`bluetoothctl disconnect`, then `connect`), wait for the daemon to redial,
and check `qs-sony status`: is multipoint still off? That is item 14 -- the
write surviving a reconnect, which another project asserts and this one has
not seen. Then turn it back on.

**6. The page.** Open jayos's Headphones page and take a screenshot. Item 13:
the page has never been drawn against a live document.

**7. The kernel.** `cargo test --test sony_link -- --ignored` binds an RFCOMM
socket and reads its address back. Item 7. It fails without a Bluetooth stack,
which is why it is ignored everywhere else.

**8. Import and settle.** Stop the daemon and `btmon`, then

    qs-sony import <dir>/trace.btsnoop --out=<dir>/imported
    qs-sony settle <dir>/imported
    qs-sony settle <dir>/record

`settle` exits 0 when every item a capture can settle is settled, 1 when some
are open (it lists them), and 2 when the directory holds no frame.

## What to commit

The imported or recorded frames go in `tests/fixtures/<model>-<firmware>/`,
for example `tests/fixtures/wh-1000xm5-2.1.0/`, with a `PROVENANCE.md` of
their own saying what was done in the session and in what order. Not into
`tests/fixtures/mdr/`, which is the WF-1000XM5 corpus and is cited by counter
throughout the code: a new capture there would renumber nothing but would mix
two headsets under one set of citations.

The frames are committed whole. `qs-sony import --exclude=PERI_RET_PARAM`
leaves out the paired-device list -- every name and address the headset
knows -- and its counter stays spent, so the gap shows. Nothing is edited
inside a frame: a capture with a byte changed is not a capture.

The tests that read `tests/fixtures/mdr/` do not read a new directory, so
nothing they pin moves. What a new capture changes is what the code may
claim: a key it holds becomes captured on the WH-1000XM5, and the tests that
say so are written against it, with its own counts.
