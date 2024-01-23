/** @type{HTMLDivElement} */
let midi_sources_div = null;

document.addEventListener('DOMContentLoaded', async () => {
	document.addEventListener('keyup', keyup);

	// Sometimes when we update the page, browser preserve the state of inputs
	// which allows us to keep keybindings from previous state of page
	for (const input of document.querySelectorAll('input.keybind')) {
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
function keyup(ev) {
	if (ev.target.nodeName == "INPUT") {
		return;
	}

	if (ev.key in registered_key_bindings) {
		console.log('registered!!!!');
	}
}

/**
	* @param {HTMLInputElement} input_element
	*/
function update_key_binding(input_element) {
	if (input_element.value.length > 0) {
		registered_key_bindings[input_element.value.trim()] = 0;
	}
}

async function change_link_status() {
	await fetch('/api/link-switch-enabled', { method: 'POST' });
}
