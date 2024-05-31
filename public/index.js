/** @type{HTMLDivElement} */
let midi_sources_div = null;


document.addEventListener('DOMContentLoaded', async () => {
	document.addEventListener('keyup', keyup);

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
	const link_status_div = document.getElementById("status");

	for (;;) {
		let socket = null;
		try {
			socket = new WebSocket(`ws://${location.host}/api/link-status-websocket`);

			socket.addEventListener("open", () => {
				console.log("successfully initialized connection with link status websocket");
				const app_health_span = document.getElementById('app-health');
				app_health_span.innerText = "Synchronized";
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
			const app_health_span = document.getElementById('app-health');
			app_health_span.innerText = "ERROR";
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

	const uuid = registered_key_bindings[ev.key];
	if (uuid) {
		await fetch(`/blocks/play/${uuid}`, { method: 'POST' });
	}
}

/**
	* @param {HTMLInputElement} input_element
	*/
function update_key_binding(input_element) {
	if (input_element.value.length > 0 && input_element.dataset) {
		console.log('Registering keybinding', input_element.value);
		const binding = input_element.value.trim();
		registered_key_bindings[input_element.value.trim()] = input_element.dataset.uuid;
		for (const other of document.querySelectorAll('input[name="keybind"]')) {
			if (other.value.trim() === binding && other.dataset.uuid !== input_element.dataset.uuid) {
				other.value = "";
			}
		}
	}
}


function toggle_delete(self) {
	document.body.classList.toggle('delete-mode-active');
}
