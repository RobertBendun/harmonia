/** @type{HTMLDivElement} */
let midi_sources_div = null;

document.addEventListener('DOMContentLoaded', () => {
	init_health_check();
});

function delay(miliseconds) {
	return new Promise(resolve => setTimeout(resolve, miliseconds));
}

async function init_health_check() {
	/** @type{HTMLSpanElement} */
	const app_health_span = document.getElementById('app-health');
	for (;;) {
		await delay(500);

		let healthy = true;
		try {
			const response = await fetch('/api/health');
			const text = await response.text();
			healthy = text == "Hi";
		} catch (err) {
			healthy = false;
		}

		if (healthy) {
			if (app_health_span.innerText != "connected") {
				app_health_span.innerText = "connected";
				app_health_span.style.color = "inherit";
			}
		} else {
			if (app_health_span.innerText != "disconnected") {
				app_health_span.innerText = "disconnected";
				app_health_span.style.color = "red";
			}
		}
	}
}

// Create WebSocket connection.
const socket = new WebSocket(`ws://${location.host}/api/ws`);

socket.addEventListener("open", (event) => {
	console.log("open", event);
});

socket.addEventListener("message", (event) => {
	console.log("message", event);
	socket.send("your message was: " + event.data);
});

socket.addEventListener("close", (event) => {
	console.log("close", event);
});

