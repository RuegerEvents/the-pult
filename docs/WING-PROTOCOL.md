# The grandMA3 command wing, over USB

What a `grandMA3 onPC command wing` says on the wire, worked out from the device and
from grandMA3 onPC 2.5.0.3 on macOS. MA publishes none of this, so everything below is
read off the hardware or off MA's own diagnostic output, and the two are kept apart:
**Observed** is something a capture shows, **inferred** is something the shape of the
evidence implies and nobody has confirmed yet.

Reverse engineering for interoperability stands on EU Software Directive art. 5(3) and
art. 6 — non-excludable under art. 8, which in German law is §69d(3), §69e and §69g(2)
UrhG — and on 17 U.S.C. §1201(f) in the United States. One part of what crosses this bus
is deliberately out of scope: the wing is also the onPC licence dongle, and
`CollectCertificates` / `SendCertificate` is that exchange. Nothing here reimplements
it, and it turned out not to be a precondition for the wing to run, so nothing needs to.

`docs/WING-NOTICE.md` says what this work contains, what it deliberately does not, and
on what footing. How the protocol was *observed* — the tracing, the instruments — is
not in this repository: it was interoperability work done once, on one machine, and it
is kept out deliberately. What is below is the interface, which is the part that has to
be written down to be maintainable.

## What it enumerates as

```
2dbe:b5c8   MA Lighting   grandMA3 onPC command wing
USB 2.0, full speed (12 Mbit/s), self-powered, 100 mA, bcdDevice 0x0100
config 1, interface 0, class FF / sub FF / proto FF
    alt 0 : 0 endpoints          <- what you see when nothing has claimed it
    alt 1 : ep 0x81 bulk IN,  64-byte packets
            ep 0x02 bulk OUT, 64-byte packets
```

No kernel driver claims it on macOS, so a plain libusb client can have it: claim
interface 0, select **alt setting 1**, then read and write those two pipes. The data
path uses no vendor control requests at all.

**Alt 0 having no endpoints is the first thing that confuses a reader.** An unclaimed
wing looks like a device with nowhere to send anything. The endpoints only exist in the
alternate setting, so a descriptor dump taken while the wing is idle describes a device
that cannot talk.

**And `SET_INTERFACE` can STALL if another host only just let go.** Right after
grandMA3 was killed, `libusb_set_interface_alt_setting(h, 0, 1)` returned
`LIBUSB_ERROR_PIPE`; the same call a minute later succeeded first time. Retry it a few
times with a short sleep rather than treating the first stall as a verdict.

MA3 ships the whole family's ids in
`shared/resource/usb_notifier.xml` — `b5c0` GMA DMX module, `b5c2` control module,
`b5c3` master module (MM), `b5c4` MFX, `b5c5` MFE, `b5c6` compact, `b5c7` compact XT,
**`b5c8` onPC command wing**, `b5c9` X-port node, `b5ca` din-rail node, `b5cb` IO node,
`b5cd` fader wing, `b5ce` onPC rack unit, `b5cf`/`b5d0` onPC DMX-key, `b510` MA-key,
`b511` viz-key.

## The wing has no application of its own

This is the fact that explains everything else about getting it running, and it is not
what you would guess from a desk that lights up when you plug it in.

What sits in the wing's flash is a **bootloader**. On every connect it asks the host for
its application, and the host streams the image over the same bulk pipe in 512-byte
packets. Observed, from MA3's own log:

```
USB: Got heartbeat with device state: 1
USB: Sending device capabilities request
USB: OnDeviceCapabilitiesDeviceType(): device type = grandMA3 onPC command wing
USB: Got heartbeat with device state: 2
USB: PC does nothing starting from this point. The initiative must be on device's side
USB: Got software request for file: command_wing_app.bin
USB: Start sending software
USB: Sending software packet for offset: 0
USB: Sending software packet for offset: 512
   ...
USB: Sending software packet (last) for offset: 71680
USB: Got heartbeat with device state: 4
```

`71680 + 320 = 72000`, which is exactly the size of
`shared/resource/software/command_wing_app.bin`. 141 packets. The heartbeat's device
state walks **1 → 2 → 3 → 4**: announced, capabilities exchanged, software loading,
running.

Three things follow.

**A host that does not send the image gets nothing.** A read-only listener on ep 0x81,
with grandMA3 not running, receives one 28-byte heartbeat a second and *nothing else* —
keys, faders and encoders produce no traffic at all. Verified over a 45-second window
with the wing being played with throughout: 45 frames, all byte-identical. The wing is
not being quiet, it is waiting to be booted.

**So a third-party host has to carry the image, and cannot ship it.** `command_wing_app.bin`
is MA's firmware. A tool should locate it in an installed grandMA3 — it is in every
version's `shared/resource/software/` — and never vendor a copy.

**And the bootloader is a different thing from the application.** `command_wing_bootloader.bin`
sits beside the app image, and MA3 carries `Allow only USB MCU update`,
`== USB DEVICE HAS BEEN UPDATED ==` and `SHUTDOWNAFTERMCUUPDATE`. Writing a *bootloader*
is the one operation on this device that could leave it unusable, and it is not on the
normal path: booting the wing happens every time you plug it in, updating its bootloader
happens when MA decides it must. See *The one way to brick it*.

## A frame is a tree of chunks

Observed, in full, from the IOKit tap. There is one rule and no exceptions:

```
chunk := u16 tag            little endian
         u16 len            little endian; bit 15 set means the body is more chunks
         len bytes body
```

The whole frame is one chunk whose tag is `0x5342` — ASCII `BS`, which is why every
frame begins `42 53`. A frame is therefore exactly `4 + len` bytes.

The "24-byte header" of the first reading was an artefact: it is not a header at all,
but the root's four bytes, plus the `0x0024` sender-address chunk with its eight bytes
of body, plus the next chunk's own four. The heartbeat in full:

```
42 53 18 80                       0x5342  container, 24 bytes
   24 00 08 00                      0x0024  len 8    sender address
      00 00 00 00 00 00 00 00         the wing sends zeros
   23 00 08 80                      0x0023  container, 8 bytes   - heartbeat
      01 00 04 00                     0x0001  len 4
         01 00 00 00                    device state = 1
```

Every frame has the same two children: `0x0024`, the sender's address, and exactly one
payload container. The address is MA's own `ID8` — the host sends
`7f 00 00 01 00 02 31 03`, which read as a little-endian `u64` is `0x033102000100007F`,
the very number MA3 prints as `IP={Local (0x033102000100007F)}`. The wing sends eight
zero bytes and never fills it in.

### The containers

| tag | direction | what it is |
|---|---|---|
| `0x0023` | both | heartbeat; `0x0001` is a `u32` device state |
| `0x0025` | host → wing | capabilities request, an empty chunk |
| `0x0026` | wing → host | capabilities reply |
| `0x0027` | both | real-time I/O — keys, faders, encoders, LEDs, DMX |
| `0x0028` | both | the application download |
| `0x0029` | both | the certificate exchange, i.e. the licence dongle. Out of scope |
| `0x002a` | wing → host | empty; sent once, at the end of the boot |

### Inside `0x0027`, which is the one that matters

| tag | dir | bytes | what |
|---|---|---|---|
| `0x0002` | in | 56 | **Key** bitmap |
| `0x0003` | in | 24 | **Fader** positions |
| `0x0004` | in | 77 | **Encoder** counters |
| `0x000f` | in | 1 | **Digital** inputs |
| `0x0012` | in | 64 | a text message from the wing |
| `0x0005` | out | 253 | **LED-Data** |
| `0x0003` | out | 24 | fader values, *written by the host* |
| `0x0001` | out | 1 | `Sync`, a rolling byte |
| `0x000a` | out | 518 / 6 | DMX frame, full and partial |

**The host writes faders as well as reading them**, on the same tag as the inbound
block. That answers the open question from the first draft of this document. The values
differ from the inbound ones by having bit 15 set — the host sends `00 90` where the
wing reports `10 00`, and `0x9000` is `0x8000 | 0x1000`. **Confirmed on the hardware**:
bit 15 means the host is driving that fader and the low twelve bits are the same scale
as the readings. These faders are **motorised**, so a word carrying the bit is a
position the motor *holds*, against a hand if need be — which is how it was confirmed,
and why a host must not set it on a fader nobody asked for. An unasserted fader sends a
plain zero.

### The boot, message by message

Every one of these is a real frame from the capture.

```
wing -> host   0x0023  device state 1              "I am here"
host -> wing   0x0025  (empty)                     capabilities request
host -> wing   0x0023  device state 2
wing -> host   0x0026  device type, capabilities   "grandMA3 onPC command wing", digital in 7
wing -> host   0x0023  device state 2
wing -> host   0x0028  0x0004 = "command_wing_app.bin"
host -> wing   0x0028  0x0001 offset, 0x0002 512 bytes     x141
wing -> host   0x0028  0x0003 16-byte progress               after each
wing -> host   0x002a  (empty)
   ... the wing restarts into the application and the whole dance runs a second time,
       this time answering 0x0027 / 0x0012 with "@No update needed" ...
wing -> host   0x0023  device state 4              running
host -> wing   0x0027  LED-Data, faders and Sync, about every 16 ms, for ever
wing -> host   0x0027  key / fader / encoder / digital blocks, when they change
```

The last line of that is why a host cannot be lazy: **the wing resets if the host stops
talking.** Stop sending and the application drops out after a few seconds and the
bootloader is back at device state 1, which is exactly what is seen if grandMA3 is quit
and a listener attached afterwards.

`'@No update needed'` arriving in a `0x0012` text chunk is also the clearest statement
available that the bootloader-update path is a *separate* decision the wing makes and
announces, not something the boot sequence walks into.

MA's own naming for this layer is visible in its symbols: `Chunk::VerifyMain illegal MainTag`,
`Chunk::Verify illegal subchunks`, `Chunk::VerifyMain wrong total size`, and
`Manet::SecureProtocolRemoteIO`. It is the same chunked RemoteIO protocol the network
wings speak, tunnelled over two bulk pipes — which is worth knowing, because it means an
Ethernet wing and a USB wing carry the same payloads inside different framing.

## Four blocks come in, one goes out

The kinds MA3 names, from its format strings: inbound `Key`, `Fader`, `Encoder`,
`Digital`, `DC`, `Midi`, `RTC`; outbound `LED-Data`, plus `Sync`, DMX frames, the
software packets above, heartbeats and the certificate exchange.

Each inbound block is a whole snapshot, sent when something in it moved. The index rules
below were each confirmed by pressing one control at a time and watching which byte
changed, then resolving the index against MA's own table (see below).

### Key — 56 bytes

A flat bitmap, and nothing more than that:

```
hardkey index = byte * 8 + bit
```

56 bytes is 448 bits; MA's table has 449 hardkey slots, of which the highest *assigned*
one is 434, so the block covers everything real.

An earlier draft put a `^ 1` word swap in this formula and was wrong on the hardware —
pressing Hilight reported `GOBACK`. The swap is real, but it belongs to reading
grandMA3's `-DEBUGUSBDATA` log, which prints the block as 16-bit groups and therefore
has its bytes the other way round from the wire; deriving the rule from the log and
then applying it to wire bytes swaps twice. **Both mistakes in this document have the
same cause**: a figure read out of grandMA3's log rendering rather than off a capture.
Five named keys settle it:

| pressed | byte | bit | `byte * 8 + bit` | MA's table |
|---|---|---|---|---|
| Hilight | 25 | 3 | 203 | `HIGHLIGHT` |
| Solo | 25 | 2 | 202 | `SOLO` |
| Blind | 27 | 4 | 220 | `BLIND` |
| Store | 19 | 3 | 155 | `STORE` |
| Go+ | 17 | 2 | 138 | `DEF_GO` |

### Fader — 24 bytes

Twelve **little-endian** `u16`, one per fader, full scale `0x1000`:

```
fader n = le16(bytes[2n], bytes[2n+1])   range 0 .. 4096, a 12-bit ADC
```

The low byte jitters by a few counts when a fader is held still, as an ADC does.

An earlier draft of this document said big-endian, and said so confidently. That came
from reading grandMA3's `-DEBUGUSBDATA` log, which prints the block as big-endian
16-bit groups; the bytes on the wire are `00 10` for a fader at full, which is
little-endian like everything else here. **The log is a rendering, the capture is the
protocol** — worth remembering for every other field in this file that was read the
same way.

### Encoder — 77 bytes

One **signed** byte per encoder definition, and it is the *turn*: how many clicks since
the wing last reported, positive one way and negative the other. **Every non-zero
reading is a turn, including one identical to the last** — the block is sent when
something moved, so two unhurried clicks the same way arrive as `1` and then `1` again,
with no zero between them to tell them apart.

An encoder has **24 detents to a full turn**, which the wire never says — it only ever
reports clicks — so anything drawing a knob needs it from somewhere. It is
`pult_wing::input::ENCODER_DETENTS`.

Both plausible alternatives are wrong, and both were tried on the desk:

- Read as a **free-running counter**, every click is followed by an equal and opposite
  event when the byte clears, and the encoder never leaves where it started — one click
  either side, for ever. The byte holding its value for up to a second between clicks is
  what makes this reading look right, and it is not.
- Read as a turn but only when the value **changes**, the second of two slow clicks is
  swallowed and so is every one after it. Turning slowly does nothing while turning fast
  works, because a quick spin lands as `2`, `3`, `5` and those differ.

### Digital — 1 byte

The wing's digital inputs. Its `HardwareIoConnectors` row says `DigitalIoPorts="7"`, and
the wing reports `Digital in capabilities: 7` over USB, so the two agree.

### LED-Data — 253 bytes, outbound

One brightness byte per LED channel, sent about every 16 ms whether or not anything
changed. An LED's byte offsets come from MA's table: single-colour LEDs carry `R` only,
31 of the 160 carry `R`, `G` and `B`. The highest offset in the table is **252** against
a 253-byte frame, which is how we know the frame is exactly the LED array and carries no
trailer.

## Being the host: five rules, each of which was a defect first

The bytes above are the easy half. Writing a host that a wing will actually run took
five corrections, and every one was found by diffing against grandMA3's own capture
rather than by reading a field. They are collected here because none of them is
discoverable from the frame formats.

**An empty container carries no sub-chunk bit.** grandMA3's capabilities request is
`25 00 00 00`, not `25 00 00 80`. Setting the bit on an empty group round-trips
perfectly through any codec and makes the wing stall its IN pipe — which is a
remarkably quiet way to be told the difference matters. `pult-wing`'s corpus test
compares what it *encodes* against grandMA3's bytes for exactly this reason: a
round-trip test would not have caught it.

**The software offset chunk is two `u32`, and the second one matters.** Offset, then a
flag that is 1 on the packet that finishes the image and 0 on every other. Without it
the wing is never told the download ended.

**Say nothing during the boot.** The wing drives it — announce, capabilities, software
request, ready — and the host only ever answers. A heartbeat injected into that
conversation stalls the pipe; an unsolicited keepalive every 500 ms breaks a download
that had otherwise worked perfectly.

**`0x002a` means let go.** After the last packet the wing restarts into what it was
given: it drops off the bus and comes back about three seconds later, and the boot runs
again — which is why grandMA3's capture has two capabilities replies, a 99-byte one
from the bootloader and a 139-byte one from the application. grandMA3 says *not one
word* in that gap. A capabilities request put there reaches a wing mid-handover, which
restarts into the bootloader instead and asks for the image again, for ever. Reconnect
by waiting for the device to actually go **absent** and return, not on a timer.

**The host is what starts it, and says so once.** On the far side the application sits
at `Loading` and stays there; what moves it to `Running` is the host sending a heartbeat
that says `Running`. The wing repeats its `Loading` heartbeat about once a second while
it waits, and answering every one of them sends two within a few hundred microseconds —
which the wing answers by dropping off the bus. Say it once.

And one that is not a rule but a relief: **the certificate exchange is not a
precondition.** grandMA3 runs it between `@No update needed` and its `Running`
heartbeat, which makes it look load-bearing. It is not — a host that skips `0x0029`
entirely gets a wing that boots, runs, lights its LEDs and reports every key. Which is
the answer to the one question this project actually had to be careful about.

**Output is one block per frame.** Faders, LEDs and sync go out as three separate
`0x0027` frames of 48, 277 and 25 bytes, never combined into one container. The
combined shape is one the wing has never been given, and it resets a few milliseconds
after receiving it.

## The control map is MA's own file, and it did not have to be guessed

```
~/MALightingTechnology/gma3_<version>/shared/resource/hardware_configurations.xml
```

It holds one `<HardwareConfiguration>` per module, and the `grandMA3 onPC command wing`
section is the index → name table for every block above:

| element | count | indexes into |
|---|---|---|
| `Hardkey` | 449 slots, 179 assigned | bits of the Key block |
| `FaderDefinition` | 12 | words of the Fader block |
| `EncoderDefinition` | 77 slots, 40 assigned | bytes of the Encoder block |
| `LedDefinition` | 158, 29 of them RGB | byte offsets in the LED frame |

Every block size matches its table exactly, which is the strongest evidence that the
indices are the wire format and not an internal detail. Six gestures on the real device
resolved like this:

| gesture | wire | table says |
|---|---|---|
| first key | byte 34 bit 3 | hardkey 283 · `EXEC` executor 201 |
| second key | byte 34 bit 2 | hardkey 282 · `EXEC` executor 202 |
| third key | byte 35 bit 2 | hardkey 274 · `EXEC` executor 203 |
| fader swept | word 1 | fader 1 · executor **202** |
| down across the sweep | byte 22 bit 6 | hardkey 190 · `FADER` executor **202** |
| encoder turned | byte 72 | encoder 72 · executor 301 |
| datawheel turned | byte 12 | encoder 12 · `Inside1` |
| datawheel pressed | byte 8 bit 6 | hardkey 78 · `ENCODER_INSIDE1` |

Two independent decodings landing on executor 202 — the fader's own word, and the
hardkey bit that was down for the whole sweep — is what makes the formulas trustworthy
rather than merely consistent. That bit is the fader's **touch contact**: MA models
touching a fader as a hardkey press, so a surface implementation gets grab and release
for free and does not have to infer them from motion.

**What the file does not contain is where anything physically is.** There are no
coordinates anywhere in MA's resources: `AnimationPos` is a boot-animation ordering, not
a position. A drawing of the panel has to be authored.

`virtual_keys.xml` beside it is the other half — what each key *means*, including what it
does under the `MA1` and `MA2` modifiers — which is the right source for a tool that wants
to label a key rather than number it.

## The one way to brick it

There is exactly one, and it is worth stating precisely because the obvious framing is
wrong.

**There is no separate "update endpoint" to stay away from.** The device has one
interface and one bulk pair, so everything — the heartbeat, the key blocks, the LED
frames, the application download and any bootloader write — travels the same two pipes.
Choosing an endpoint protects nothing. What protects the device is the *content* of what
is sent.

The rules that follow:

- **Send only chunk shapes that have been watched going to a wing.** Never invent a tag
  and never sweep a tag space. A vendor's unknown command space is exactly where
  "erase flash" lives.
- **Booting the wing is not flashing it.** `command_wing_app.bin` is downloaded on every
  single connect, which is not something a device does to its own flash. Sending it is
  ordinary traffic.
- **Never send `command_wing_bootloader.bin`, and never interrupt anything that looks
  like it is writing one.** An interrupted bootloader write is the unrecoverable case.
- **Keep one grandMA3 version installed** while working on this, and note `bcdDevice`
  (`0x0100` here) so a firmware change is visible if one ever happens.
- Reading is always safe. A listener on ep 0x81 cannot do any harm.

## What is still unknown

Most of this list was the whole document a day ago, and playing the desk has closed
most of the rest — the encoder byte and the outbound fader bit were both settled by
somebody turning and pushing things. What is left:

- **The `MA` key.** The panel has one between Go+ and Learn, and MA's table has `MA1`
  and `MA2`. Whether either produces a bit in the key block is untested; if neither
  does, the wing resolves the modifier itself and never puts it on the wire.
- **Several capability fields.** `0x0026` carries `0x0009`, `0x0013`, `0x000b`, `0x000c`
  and `0x0002` alongside the ones MA3 names in its log, and nothing says what they mean.
- **The `Sync` byte** (`0x0027` / `0x0001`), which rises by one per frame and is probably
  exactly that.
- **The `DC`, `Midi` and `RTC` blocks.** Named in the binary, never seen on this wing.
- **The 16-byte software progress chunk** `0x0028` / `0x0003`, only the first four bytes
  of which are obviously an offset.
- **Everything about `0x0029`**, deliberately.
