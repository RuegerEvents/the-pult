# About the grandMA3 wing support

`crates/pult-wing` and `tools/wing-explorer` let this console talk to a grandMA3
command wing. This note says what that work is, what it deliberately is not, and what
it contains.

The instruments the protocol was worked out with are **not in this repository**. They
read a licensed grandMA3 installation and trace its traffic — interoperability work,
done once, on one machine — and they are gitignored rather than published. What is here
is the driver and the tool, which is the part anybody else would want.

**No affiliation.** MA Lighting Technology GmbH has no involvement in this and has not
endorsed it. *grandMA*, *grandMA3* and *MA Lighting* are their trademarks, used here
only to say which hardware this interoperates with.

## What is here

Everything under those three directories is original work, MIT-licensed with the rest of
the repository: a chunk codec, block decoders, a USB transport, a drawing of the panel
and some instruments that were used to work the protocol out.

`docs/WING-PROTOCOL.md` describes an interface — what bytes mean on a wire between two
devices. Interface facts are not the software that implements them.

## What is deliberately not here

**MA's firmware.** The wing holds only a bootloader and asks the host for
`command_wing_app.bin` at every connect. That file is MA's. `wing-explorer` locates it
in a grandMA3 installation on the machine it runs on and never carries a copy. Running
the tool therefore requires a licensed grandMA3 of your own.

**MA's control map.** `hardware_configurations.xml` is MA's; it is read from a local
installation by `tools/wing-explorer/extract-wing-map.py`, and the generated
`wing-map.json` is gitignored rather than committed.

**The certificate exchange.** Container `0x0029` is the onPC licence dongle. It is named
in the documentation so that it can be recognised and skipped, and it is not implemented
— nothing here helps anybody run MA software they have not licensed. It also turned out
not to be a precondition for the wing to work, so there was never a reason to go near
it.

**MA's code, in the test corpus.** `testdata/wing-frames.json` holds twenty captured
frames, which exist to pin the framing. Two of them are software packets, whose payload
on the wire is MA's firmware; that payload has been replaced with a synthetic pattern of
the same length. The frames still prove everything they were there to prove. What
remains that could be matched against MA's binary is the wing's own identification
strings — `grandMA3 onPC command wing`, `@No update needed` — which the device sends,
and which are protocol content.

## The legal footing, as we understand it

This is interoperability work: making our own console talk to hardware its owner already
has. In the EU that is expressly protected. Directive 2009/24/EC Art. 5(3) covers
observing, studying and testing a program you are licensed to run, and Art. 6 covers
going further where it is indispensable to obtain interoperability information — and
Art. 8 makes both non-excludable, which in German law is §69d(3), §69e and §69g(2) UrhG.
In the United States the corresponding carve-out is 17 U.S.C. §1201(f).

MA's own EULA prohibits reverse engineering and then carves out exactly this: *"except
as and only to the extent any foregoing restriction is prohibited by applicable law"*.

The conditions that come with those rights are met here: the work was done by a
licensee, on a licensed installation, the information is not otherwise available, it
went no further than was needed to talk to the hardware, and none of it is used to build
a program substantially similar to grandMA3 onPC — `pult-wing` is a driver for a control
surface, not a lighting console.

None of that is legal advice, and none of it is a promise that MA will be pleased.
