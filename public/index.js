/** @type{HTMLDivElement} */
let midi_sources_div = null;


document.addEventListener('DOMContentLoaded', async () => {
	document.addEventListener('keyup', keyup);

	set_color_scheme(get_color_scheme());

	// Sometimes when we update the page, browser preserve the state of inputs
	// which allows us to keep keybindings from previous state of page
	for (const input of document.querySelectorAll('input[name=keybind]')) {
		update_key_binding(input);
	}

	await init_websocket();
});

function delay(miliseconds) {
	return new Promise(resolve => setTimeout(resolve, miliseconds));
}

/**
	* @param {'open'|'close'} type
	* @param {WebSocket} socket
	*/
function socket_event(type, socket) {
	return new Promise(resolve => socket.addEventListener(type, resolve));
}

async function init_websocket() {
	const link_status_div = document.getElementById("link-status");
	const app_health_span = document.getElementById('app-health');

	for (;;) {
		let socket = null;
		try {
			socket = new WebSocket(`ws://${location.host}/api/link-status-websocket`);

			socket.addEventListener("open", () => {
				console.log("successfully initialized connection with link status websocket");
				app_health_span.innerText = "connected";
				app_health_span.style.color = "inherit";
			});

			socket.addEventListener("message", (event) => {
				link_status_div.innerHTML = event.data;
			});
		} catch (err) {
			console.error(err);
		} finally {
			if (socket) {
				await socket_event("close", socket);
			}
			console.error("Connection was closed, trying to reconnect after 300ms");
			app_health_span.innerText = "disconnected";
			app_health_span.style.color = "red";

			await delay(300);
		}
	}
}

// TODO: When refreshing page previous value of keybind cell may stay,
//       but we only notice when page changes. Also it should be preserved
//       across the calls, so send this keybindings to server.
const registered_key_bindings = {};

/**
	* @param {KeyboardEvent} input_element
	*/
async function keyup(ev) {
	if (ev.target.nodeName == "INPUT") {
		return;
	}

	const input_element = registered_key_bindings[ev.key];
	if (input_element) {
		await fetch(`/midi/play/${input_element.dataset.uuid}`, { method: 'POST' });
	}
}

/**
	* @param {HTMLInputElement} input_element
	*/
function update_key_binding(input_element) {
	if (input_element.value.length > 0) {
		console.log('Registering keybinding', input_element.value);
		registered_key_bindings[input_element.value.trim()] = input_element;
	}
}

async function change_link_status() {
	await fetch('/api/link-switch-enabled', { method: 'POST' });
}

function get_color_scheme() {
	let scheme = window.localStorage.getItem('preffered-color-scheme');
	if (scheme) {
		return scheme;
	}
	if (window.matchMedia) {
		scheme = window.matchMedia('(prefers-color-scheme: light)').matches ? "light" : "dark";
	} else {
		scheme = "dark";
	}
	window.localStorage.setItem('preffered-color-scheme', scheme);
	return scheme;
}

function set_color_scheme(scheme) {
	document.documentElement.style.colorScheme = scheme;
	window.localStorage.setItem('preffered-color-scheme', scheme);
	// const color_switch = document.getElementById('color-switch');
	// if (color_switch) {
	// 	color_switch.innerText = icon[scheme];
	// }
}

function toggle_color_scheme() {
	set_color_scheme({ "light": "dark", "dark": "light" }[get_color_scheme()])
}
