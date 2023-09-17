/** @type{HTMLDivElement} */
let midi_sources_div = null;

document.addEventListener('DOMContentLoaded', () => {
	const input = document.getElementById('midi-sources-input');
	input.addEventListener('change', async function() {
		/** @type{FileList} */
		const fileList = this.files;

		const formData = new FormData();
		for (const file of fileList) {
			formData.append('files[]', file);
		}
		const response = await fetch('/api/midi/add', { method: 'POST', body: formData });
		for (const midi_source of await response.json()) {
			const tile = document.createElement('div');
			if (midi_source.Ok) {
				const { file_name, format, tracks_count, uuid } = midi_source.Ok;
				tile.setAttribute('data-uuid', uuid);
				tile.innerHTML = `<h3>${file_name}</h3><p>Format: ${format}, tracks count: ${tracks_count}</p>`;
				const btn = document.createElement('button');
				btn.innerText = 'Play';
				btn.addEventListener('click', async () => {
					const response = await fetch(`/api/midi/play/${uuid}`, { method: 'POST' });
					const data = await response.text();
					console.log(data);
				});
				tile.appendChild(btn);
			} else {
				tile.innerHTML = `<strong>ERROR</strong>: ${midi_source.Err}`;
			}
			midi_sources_div.appendChild(tile);
		}
	});

	midi_sources_div = document.getElementById('midi-sources-list');
});

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

