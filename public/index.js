/** @type{HTMLDivElement} */
let midi_sources_div = null;

document.addEventListener('DOMContentLoaded', async () => {
	await Promise.all([
		init_health_check(),
		init_websocket(),
	]);
});

function delay(miliseconds) {
	return new Promise(resolve => setTimeout(resolve, miliseconds));
}

// TODO: Is this needed when we have constant communication using Websockets?
//       This is probably just unnessesary noise (and traffic).
//       Websocket restart logic should handle this status rendering
async function init_health_check() {
	/** @type{HTMLSpanElement} */
	const app_health_span = document.getElementById('app-health');
	for (;;) {
		await delay(500);

		try {
			const timeout = 100;
			const abort = new AbortController();
			const timeout_id = setTimeout(() => abort.abort(), timeout);

			const response = await fetch('/api/health', { signal: abort.signal });
			clearTimeout(timeout_id);
			const text = await response.text();
			if (text != "Hi") { throw new Error(`expected health check to return "Hi", but it returned: "${text}"`); }

			if (app_health_span.innerText != "connected") {
				app_health_span.innerText = "connected";
				app_health_span.style.color = "inherit";
			}
		} catch (err) {
			if (app_health_span.innerText != "disconnected") {
				console.error(err);
				app_health_span.innerText = "disconnected";
				app_health_span.style.color = "red";
			}
		}
	}
}

/**
	* @param {WebSocket} socket
	*/
function socket_closed(socket) {
	return new Promise(resolve => socket.addEventListener('close', resolve));
}

async function init_websocket() {
	const link_status_div = document.getElementById("link-status");

	for (;;) {
		const socket = new WebSocket(`ws://${location.host}/api/link-status-websocket`);
		socket.addEventListener("open", () => {
			console.log("successfully initialized connection with link status websocket");
		});

		socket.addEventListener("message", (event) => {
			link_status_div.innerHTML = event.data;
		});

		await socket_closed(socket);
		console.error("Connection was closed, trying to reconnect after 0.5s");
		await delay(300);
	}
}
