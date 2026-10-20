/**
 * The icons a plugin may name in its manifest.
 *
 * A fixed list rather than all of Lucide, deliberately: a panel names its icon at
 * build time and the bundler keeps exactly those, but a plugin names one at *run*
 * time, and reaching every Lucide icon by name would put a few thousand of them into
 * the binary that serves this page. So a console carries this short list — the ones a
 * plugin is likely to want — and a name it does not know is drawn as the puzzle piece
 * every plugin gets by default. `docs/PLUGINS.md` lists the names.
 */

import type { Component } from 'svelte';

import Activity from '@lucide/svelte/icons/activity';
import AudioWaveform from '@lucide/svelte/icons/audio-waveform';
import Bot from '@lucide/svelte/icons/bot';
import Cable from '@lucide/svelte/icons/cable';
import Gauge from '@lucide/svelte/icons/gauge';
import Lightbulb from '@lucide/svelte/icons/lightbulb';
import MessageSquare from '@lucide/svelte/icons/message-square';
import Mic from '@lucide/svelte/icons/mic';
import Network from '@lucide/svelte/icons/network';
import Puzzle from '@lucide/svelte/icons/puzzle';
import ScrollText from '@lucide/svelte/icons/scroll-text';
import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
import Sparkles from '@lucide/svelte/icons/sparkles';
import Terminal from '@lucide/svelte/icons/terminal';
import Timer from '@lucide/svelte/icons/timer';
import WandSparkles from '@lucide/svelte/icons/wand-sparkles';
import Workflow from '@lucide/svelte/icons/workflow';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const PLUGIN_ICONS: Record<string, Component<any>> = {
	activity: Activity,
	'audio-waveform': AudioWaveform,
	bot: Bot,
	cable: Cable,
	gauge: Gauge,
	lightbulb: Lightbulb,
	'message-square': MessageSquare,
	mic: Mic,
	network: Network,
	puzzle: Puzzle,
	'scroll-text': ScrollText,
	'sliders-horizontal': SlidersHorizontal,
	sparkles: Sparkles,
	terminal: Terminal,
	timer: Timer,
	'wand-sparkles': WandSparkles,
	workflow: Workflow
};

/** The icon a plugin named, or the puzzle piece. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const pluginIcon = (name: string | null | undefined): Component<any> =>
	(name && PLUGIN_ICONS[name]) || Puzzle;
