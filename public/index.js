/** @type{HTMLDivElement} */
let midi_sources_div = null;

document.addEventListener('DOMContentLoaded', async () => {
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
