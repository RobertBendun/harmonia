// Create WebSocket connection.
const socket = new WebSocket(`ws://${location.host}/ws`);

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

