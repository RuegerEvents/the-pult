/**
 * The worked example of a plugin-shipped panel: a custom element, served by
 * the backend from this plugin's assets, mounted into a tile by the console.
 *
 * Before the element is attached, the console sets `this.pult` — the bridge:
 *
 *   pult.call(method, args)      → this plugin's rpc.handle
 *   pult.get(path)               → one read of the show
 *   pult.subscribe(pattern, cb)  → live updates; returns an unsubscribe fn
 *
 * This one watches the programmer: every value the operators are holding,
 * however it got there — values panel, command line, or another console.
 */

class PultCliMonitor extends HTMLElement {
	connectedCallback() {
		this.attachShadow({ mode: 'open' });
		this.shadowRoot.innerHTML = `
			<style>
				:host { display: block; height: 100%; overflow-y: auto; }
				.wrap { padding: 10px 12px; font: 12px ui-monospace, Menlo, monospace; color: #bbb; }
				h3 { margin: 0 0 8px; font-size: 11px; font-weight: 600;
				     text-transform: uppercase; letter-spacing: 0.06em; color: #777; }
				table { border-collapse: collapse; width: 100%; }
				td { padding: 2px 10px 2px 0; white-space: nowrap; }
				td.v { color: #eee; }
				.locked { color: #d9a; }
				.empty { color: #666; font-style: italic; }
			</style>
			<div class="wrap">
				<h3>Programmer</h3>
				<div id="body" class="empty">nothing held</div>
			</div>`;
		this.refresh();
		this.stop = this.pult.subscribe('programmer_values/**', () => this.refresh());
	}

	disconnectedCallback() {
		this.stop?.();
	}

	async refresh() {
		const entries = (await this.pult.get(['programmer_values'])) ?? [];
		const body = this.shadowRoot.getElementById('body');
		if (!entries.length) {
			body.className = 'empty';
			body.textContent = 'nothing held';
			return;
		}
		body.className = '';
		const row = (e) => {
			const kind = typeof e.parameter_kind === 'string'
				? e.parameter_kind
				: Object.keys(e.parameter_kind)[0];
			const value = e.value?.type === 'Float'
				? `${Math.round(e.value.value * 100)}%`
				: JSON.stringify(e.value?.value);
			const lock = e.locked ? ' <td class="locked">locked</td>' : '<td></td>';
			return `<tr><td>${e.fixture_id.slice(0, 8)}</td><td>${kind}</td><td class="v">${value}</td>${lock}</tr>`;
		};
		body.innerHTML = `<table>${entries.map(row).join('')}</table>`;
	}
}

customElements.define('pult-cli-monitor', PultCliMonitor);
