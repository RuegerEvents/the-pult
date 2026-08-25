# The Pult

> new, intuitive Lighting Console for everyone (consumer to professional) with rethinked workflows, easy integrations and a modern stack, look and feel and feature set

![](https://md.service42.me/uploads/upload_77261a39582ecdd8408457b622e92bc9.png)


## Basics
- Programmer & Playback are useful principles, are kept

## Programmer
- main 3D view of the rig
- allows user to add products: fixtures, trusses, stage plattforms, etc
- everything needs a position, but does not have to be exact at the beginning
- standard view from FOH perspective (other standard positions can exist)
- can drag away, but when releasing mouse/finger, snaps back into original view
- poles, walls, etc must be semi-transparent if there are fixtures behind


### Programming
- Happens at the fixture: select a fixture -> 3D view is zoomed to the fixture
- Popup opens with the attributes of the fixture (not as a list, but as a puppeteering of the fixture)
- user can grab the pan or tilt axis and it results in the movement of that axis, just like it would behave in real life
- user can also select an effector (e.g. light source of that fixture) and gets a quicksheet (similar to ETC) where all attributes that change the properties of that effector can be manipulated
- pop-up on the right with current programmer values, can be swiped to see the list of parameters, can be cleared with swipe gestures or a clear all button
- can be locked to keep them without saving (parking function) to be saved in multiple sequences without the need of a store menu

### Store Menu
- shows which parameters are currently stored, with a clear overview of which fixtures, attributes and values are stored
- user can deselect parameters to not store them

### Selection
- separate from programmer
- is kept when programmer is cleared, and programmer is kept when selection is cleared
- shown in a panel on the right side, similar to Blender's settings panel
- shows how many fixtures are currently in the selection
- fixtures in the selection can be reordered through drag and drop or removed from the selection
- selection can be defined through geometric functions, e.g. radial selection by defining a starting point and an angle, which selects fixtures within that area
- --> selection is always dynamicly generated based on the current rig and the defined geometric functions, not a static list of fixtures (useful for festivals, changing fixtures, etc)

### Effects / Phasers
- no concept of selection grid, but always derived from the selection in 3d with modifiers (symmetrical, groups, etc)
- can be fixture-based or global/rig-based (e.g. all Titan Tubes run the same effect or the effect runs accross the rig)
- modifiers can be defined statically (e.g. 2 or 3), but also dynamically based on the rig (e.g. number of fixtures, number of instances of a fixture, etc)

### Fixtures & Discovery
- network-capable fixtures appear in a panel on the left side in a swipeable panel
- can be added or matched to existing fixtures in the rig through drag and drop
- attributes, colors, intensities must be generic
- positions must be either axial (position vector and direction vector) or positional (XYZ)

### Designer-Fader
- show a current live view of attributes of a fixture group
- allows live manipulation, but moves once a cue changes that attribute
- when touch-sensitive fader and operator holds it while a cue would change that attribute, the attribute is not changed until the operator releases the fader, allowing for temporary overrides of cues

## Playback

### Playback Philosophy
- busking workflows should not require manually building hundreds of sequences or macro-based start show files, but rather be dynamic
- playback structures should be dynamically generated from available fixtures, groups, colors and attributes
- standard playback surfaces should be generatable automatically instead of manually assembled
- user should still be able to create fully custom workflows if desired

### Layout Views
- MA-like layout views are not needed for fixture selection (selections are generated geometrically from 3D)
- are needed for live operating
- provide generators for: color picker, device-based control surfaces, delay and direction controls, effect parameter controls, etc
- be fully dynamic and re-render based on rig, capabilities and user preferences
- allow custom uis via web components

### Staging / Preview Concept
- playback should support preparing combinations of sequences before triggering them live
- operators should be able to stage multiple changes together and execute them simultaneously
- preview visualization for staged changes before going live

### Output System & Data Handling
- fixtures internally behave like intelligent processing nodes
- output plugins translate high-level effect data into protocol-specific output (e.g. DMX at 40 Hz)
- optional DMX output layer handles fixture-specific calculations and limitations
- fixtures may preload upcoming playback data where possible
- effects should send only parameter changes instead of continuous full-frame updates
- many effects (e.g. strobes) can be represented by compact parameter descriptions like rate, duration, modifiers
- more complex effects (e.g. random strobes) may require standardized implementations inside fixtures or realtime data transmission from the console
- system can preload likely upcoming playback content (timecode-based, cuestack-based, etc)
- visible or currently accessible playback controls may be prioritized for preloading
- concept similar to optimistic loading in frontend systems

### Timecode & Audio Synchronization

#### Timecode Workflow
- timecode should be waveform-based
- supports markers, beat grids, quantization
- attach playback events directly to waveform positions instead of to timecode values

#### "Timecode without Timecode"
- concept for synchronizing playback to live bands without click tracks
- based on prerecorded reference tracks, realtime audio analysis and beat and pattern matching
- system maintains beat synchronization, estimate playback position in realtime
- still allow operator correction when drift occurs
- possible use cases: automatic speed master synchronization, semi-automatic busking support, live music synchronization without dedicated timecode

#### Speedmaster Improvements
- speedmasters should explicitly track the musical "one" (beginning of a measure)
- effects should support configurable phase origins
- supports: center-origin effects, reversed timing, pickups / upbeats, custom phase alignment

### Fixture Positions, Tracking & Spatial Awareness
- future system should integrate tracking and positional feedback directly
- fixture positions may be manually defined, calibrated automatically and can be updated dynamically
- enables deeper integration with moving systems and tracked environments
- could replace external tracking integration layers like PSN in some workflows
- system should internally combine fixture state, tracking data, 3D calculations and playback logic

## Event-Based Control & Automation
- The Pult is not limited to lighting control
- support FX control, automation, interactive installations, exhibition workflows and sensor-driven experiences
- all connected devices should be network-capable: props, sensors, relays, audio players, lighting devices and automation systems

### Event System
- support event-driven workflows in addition to traditional playback
- events can originate from sensors, external systems, network protocols, automation systems and user-defined triggers
- users can define logic such as triggering audio, lighting cues or automation actions after delays or external events
- playback can become reactive instead of purely timeline-based

### Node-Based Workflow
- optional node-editor-style workflow system
- visually connect triggers, events, playback actions and automation logic
- node-based and cue-based trigger workflows may coexist

### Audio, Video & Synchronization
- integrated audio playback engine
- playback may also happen through synchronized external systems or via timecode
- video remains only external, can be triggered

## Architecture & Technology Stack

### Frontend
- frontend should be fully web-based
- supports desktop, tablet, phone
- frontend rendering happens locally on client devices
- console should not stream rendered UI images

### Backend
- backend should use a performant systems language (likely Rust)
- backend nodes can operate as distributed processing nodes
- all backend nodes share synchronized show state

### Synchronization Model
- backend nodes maintain the complete synchronized show state
- frontend clients subscribe only to relevant data where possible
- large 3D visualizations may eventually require backend-side rendering or preprocessing for scalability
- especially relevant for very large rigs (e.g. ESC-scale productions)

### 3D Rendering
- FOH perspective remains the primary operational perspective
- 3D view is the most state-heavy part of the system
- future optimizations may move heavy calculations away from frontend clients

## General principles
### Custom Interfaces & Extensibility
- open interfaces (OSC, MIDI, WebSocket, etc), also for direct manipulation of values
- some kind of plugin system
- control surface interfaces
- external interfaces could also live in building automation systems, wall panels, tablets, etc
- modern network-based communication preferred over DMX-centric workflows

## Inspiration
![](https://md.service42.me/uploads/upload_cea30b9aa0443579588a7008e19f2c75.png)

![](https://md.service42.me/uploads/upload_aba026fc10f5c0dae657608efe3a6eaa.png)

![](https://md.service42.me/uploads/upload_6ad87a8b7cd68d07a46036828f739526.png)

![](https://md.service42.me/uploads/upload_8d0c60550a6364034f5b8436bd4815c4.png)

![](https://md.service42.me/uploads/upload_65a3f57323d63b9828e84da4925c176c.png)

![](https://md.service42.me/uploads/upload_350f296999a5741f6fa2c946be10296b.png)

![](https://md.service42.me/uploads/upload_23d212288172907195a3993da9c3496f.png)

![](https://md.service42.me/uploads/upload_38533bb93c56c662aa2898ec315cff19.png)

![](https://md.service42.me/uploads/upload_1cd0be3c19ac86e85ffce15fd9415ccd.png)

![](https://md.service42.me/uploads/upload_19b7ad751ef1839977decf3031110d08.png)

![](https://md.service42.me/uploads/upload_f2bf392add2429b1f2328b4ec1537c11.png)

![](https://md.service42.me/uploads/upload_ca7e797bb7b0f4402b7dc6d76a8b989c.png)
