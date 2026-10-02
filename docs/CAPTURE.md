# Capturing a WH-1000XM5

One session with the headset on a desk settles most of `docs/OUTSTANDING.md`.
`tools/acceptance.sh` runs it and waits where a person is needed; this is what
it does and why each step is where it is. Nothing here has been run: there is
no adapter where it was written, and the script has only run as `--dry-run`.

## What it needs

- A WH-1000XM5, paired and connected, and nothing else talking to it: Sony's
  app on a phone holds the headset's one control channel.
- A second device -- a phone, with Sony's app closed -- connected to the
  headset at the same time, multipoint on, for item 6.
- BlueZ, with `btmon` (bluez-utils) and root to run it.
- This package installed, or a checkout's `target/release` on `PATH`.
- `cargo`, for the one test that needs a Bluetooth stack.
- `jq`, which `tools/acceptance.sh` reads `qs-sony devices` with.
- jayos: its `qs-devices` is the inventory `qs-sony devices` reads, and the
  script will not start without it naming a WH-1000XM5; and the screenshot is
  of its page.

## The session

**1. Trace first.** `btmon -w <dir>/trace.btsnoop` as root, before the daemon
dials, so the trace holds the L2CAP signalling that names the RFCOMM channel
and the whole handshake. A trace started later still imports -- a dynamic
channel no signalling in the file names is tried as RFCOMM, link by link, so
the reconnect in step 5 does not cost the first link's frames -- but it
starts mid-conversation, without the handshake.

Then connect the second device as well, multipoint on (item 6): the headset's
paired-device list, asked when the daemon connects, will show two peers
connected, and that is the one reading that says which byte means ON.

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
alert yes` answers. The header does not say WHICH link a disconnecting message
drops -- turning multipoint off may drop the phone rather than this machine.
If it drops this one while the answer is on the wire or within five seconds of
the headset acknowledging it, the daemon records a clean close; a drop after
that, or none at all, is whatever the transport says it was.

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
are open or contradicted (it lists them; contradicted means the capture
settles the item against this package's reading), and 2 when nothing was
examined -- the directory holds no frame, or is not there to read.

`import` reports `frames` (every frame-shaped span on the channel),
`not_frames` (spans among them that do not unpack: counted, not written, so
their counters leave gaps), `unfinished` (frames a link ended in the middle
of) and `written`.

## What to commit

The imported or recorded frames go in `tests/fixtures/<model>-<firmware>/`,
for example `tests/fixtures/wh-1000xm5-2.1.0/`, with a `PROVENANCE.md` of
their own saying what was done in the session and in what order. Not into
`tests/fixtures/mdr/`, which is the WF-1000XM5 corpus and is cited by counter
throughout the code: a new capture there would renumber nothing but would mix
two headsets under one set of citations.

The frames are committed whole. `qs-sony import
--exclude=PERI_RET_PARAM,PERI_NTFY_PARAM` leaves out the paired-device list --
every name and address the headset knows -- asked for and volunteered, the
same body under two commands; their counters stay spent, so the gaps show.
`PERI_NTFY_EXTENDED_PARAM` carries one address and no names, and can be
added. A name neither command table declares is refused, so a typo cannot
quietly exclude nothing. Excluding the list loses item 6's evidence from the
committed frames; that is the trade. Nothing is edited inside a frame: a
capture with a byte changed is not a capture. The recording `qs-sonyd
--record` writes has no `--exclude`; it is the private copy.

The tests that read `tests/fixtures/mdr/` do not read a new directory, so
nothing they pin moves. What a new capture changes is what the code may
claim: a key it holds becomes captured on the WH-1000XM5, and the tests that
say so are written against it, with its own counts.
