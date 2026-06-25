#include "pipewire/stream.h"
#include <errno.h>
#include <libgen.h>
#include <pipewire/pipewire.h>
#include <spa/utils/result.h>
#include <spa/pod/builder.h>
#include <spa/pod/vararg.h>
#include <spa/control/control.h>
#include <spa/param/format.h>
#include <stdlib.h>
#include <string.h>


#define NOB_IMPLEMENTATION
#include "nob.h"

#include <midifile.h>
#include <midievent.h>

struct node_info
{
	size_t node_id;
	char const* node_name;
};

struct port_info
{
	size_t node_id;
	char const* port_name;
	char const* port_serial;
};

struct userdata
{
	struct pw_main_loop *loop;
	struct pw_context *context;
	struct pw_core *core;
	struct pw_registry *registry;

	struct spa_hook core_listener, registry_listener, stream_listener;

	int sync;

	struct
	{
		struct port_info *items;
		size_t count, capacity;
	} found_ports;

	struct
	{
		struct node_info *items;
		size_t count, capacity;
	} found_nodes;

	struct midi_file *midi_file;
	struct midi_file_info midi_file_info;

	struct pw_stream *stream;
	uint64_t clock_time;
	struct spa_io_position *position;
};

static void do_quit(void *userdata, int signal_number)
{
	(void)signal_number;
	struct userdata *data = userdata;
	pw_main_loop_quit(data->loop);
}


static void on_core_done(void *userdata, uint32_t id, int seq)
{
	(void)id;
	struct userdata *data = userdata;
	if (data->sync != seq) return;

	pw_main_loop_quit(data->loop);
}

static void on_core_error(void *data, uint32_t id, int seq, int res, const char *message)
{
	struct userdata *d = data;

	pw_log_error("error id:%u seq:%d res:%d (%s): %s",
			id, seq, res, spa_strerror(res), message);

	if (id == PW_ID_CORE && res == -EPIPE)
		pw_main_loop_quit(d->loop);
}

static void registry_event_global(
	void *userdata,
	uint32_t id,
	uint32_t permissions,
	const char *type,
	uint32_t version,
	const struct spa_dict *props
)
{
	(void)permissions;
	(void)version;

	struct userdata *data = userdata;

	if (props == NULL) return;

	if (strcmp(type, PW_TYPE_INTERFACE_Port) == 0) {
		char const* direction = spa_dict_lookup(props, PW_KEY_PORT_DIRECTION);
		if (!direction || strcmp(direction, "in") == 0)
			return;

		char const* format = spa_dict_lookup(props, PW_KEY_FORMAT_DSP);
		if (!format || strcmp(format, "8 bit raw midi") != 0)
			return;

		char const* serial = spa_dict_lookup(props, PW_KEY_OBJECT_SERIAL);
		if (!serial)
			return;

		char const* name = spa_dict_lookup(props, PW_KEY_PORT_NAME);
		if (!name)
			return;

		char const* node_id = spa_dict_lookup(props, PW_KEY_NODE_ID);
		if (!node_id)
			return;

		// struct spa_dict_item const* item;
		// spa_dict_for_each(item, props) {
		// 	printf("Port[%d][%s] = %s\n", id, item->key, item->value);
		// }

		struct port_info port = {
			.port_name = strdup(name),
			.port_serial = strdup(serial),
			.node_id = atoi(node_id),
		};
		nob_da_append(&data->found_ports, port);

		return;
	}

	if (strcmp(type, PW_TYPE_INTERFACE_Node) == 0) {
		char const* name = spa_dict_lookup(props, PW_KEY_NODE_NAME);
		if (!name)
			return;

		struct node_info node = {
			.node_id = id,
			.node_name = strdup(name),
		};
		nob_da_append(&data->found_nodes, node);

		// struct spa_dict_item const* item;
		// spa_dict_for_each(item, props) {
		// 	printf("Node[%d][%s] = %s\n", id, item->key, item->value);
		// }
		return;
	}
}

static int midi_play(struct userdata *d, void *src, unsigned int n_frames)
{
	int res;
	struct spa_pod_builder b = {};
	struct spa_pod_frame f;
	uint32_t first_frame, last_frame;
	bool have_data = false;

	spa_pod_builder_init(&b, src, n_frames);
	spa_pod_builder_push_sequence(&b, &f, 0);

	first_frame = d->clock_time;
	last_frame = first_frame + d->position->clock.duration;
	d->clock_time = last_frame;

	for (;;) {
		uint32_t frame;
		struct midi_event ev;
		size_t size;

		res = midi_file_next_time(d->midi_file, &ev.sec);
		if (res <= 0) {
			if (have_data)
				break;
			return res;
		}

		frame = (uint32_t)(ev.sec * d->position->clock.rate.denom);
		if (frame < first_frame)
			frame = 0;
		else if (frame < last_frame)
			frame -= first_frame;
		else
			break;

		midi_file_read_event(d->midi_file, &ev);
		midi_event_dump(stderr, &ev);

		spa_pod_builder_control(&b, frame, SPA_CONTROL_Midi);
		spa_pod_builder_bytes(&b, ev.data, ev.size);

		size = ev.size;

		if (ev.type != MIDI_EVENT_TYPE_MIDI1 || size < 1 || ev.data[0] == 0xff)
			continue;

		have_data = true;
	}
	spa_pod_builder_pop(&b, &f);

	return b.state.offset;
}

void on_process(void *userdata)
{
	struct userdata *data = userdata;
	struct pw_buffer *b;
	struct spa_buffer *buf;
	struct spa_data *d;
	uint8_t *p;

	// Get buffer that we can fill with data
	if ((b = pw_stream_dequeue_buffer(data->stream)) == NULL)
		return;

	buf = b->buffer;
	d = &buf->datas[0];

	bool have_data = false;

	if ((p = d->data) == NULL)
		return;

	int n_frames = d->maxsize;
	if (b->requested)
		n_frames = SPA_MIN(n_frames, (int)b->requested);

	int n_fill_frames = midi_play(data, p, n_frames);
	if (n_fill_frames > 0 || n_frames == 0) {
		d->chunk->offset = 0;
		d->chunk->stride = 1;
		d->chunk->size = n_fill_frames;
		have_data = true;
		b->size = n_fill_frames;
	}

	if (have_data)
		pw_stream_queue_buffer(data->stream, b);
	else
		pw_stream_flush(data->stream, true);
}

static void on_io_changed(void *userdata, uint32_t id, void *data, uint32_t size)
{
	(void)size;
	struct userdata *d = userdata;

	switch (id) {
	case SPA_IO_Position:
		d->position = data;

		char const* state = "(undefined)";
		switch (d->position->state) {
		case SPA_IO_POSITION_STATE_RUNNING: state = "running"; break;
		case SPA_IO_POSITION_STATE_STARTING: state = "starting"; break;
		case SPA_IO_POSITION_STATE_STOPPED: state = "stopped"; break;
		}

		nob_log(NOB_INFO,
				"Position changed:"
				" clock.clock.rate=%u/%u"
				" clock.duration=%lu"
				" state=%s"
				, d->position->clock.rate.num, d->position->clock.rate.denom
				, d->position->clock.duration
				, state
				);
		break;

	default:
		break;
	}
}

static void on_state_changed(void *userdata, enum pw_stream_state old, enum pw_stream_state state, const char *error)
{
	struct userdata *data = userdata;
	nob_log(NOB_INFO, "stream state changed %s -> %s", pw_stream_state_as_string(old), pw_stream_state_as_string(state));

	switch (state) {
	case PW_STREAM_STATE_ERROR:
		nob_log(NOB_ERROR, "stream at node %d failed: %s\n", pw_stream_get_node_id(data->stream), error);
		pw_main_loop_quit(data->loop);
		break;

	case PW_STREAM_STATE_PAUSED:
	case PW_STREAM_STATE_CONNECTING:
	case PW_STREAM_STATE_UNCONNECTED:
	case PW_STREAM_STATE_STREAMING:
		break;
	}
}

static void on_drained(void *userdata)
{
	struct userdata *data = userdata;
	pw_main_loop_quit(data->loop);
}

int main(int argc, char **argv)
{
	int exit_code = 0;
	struct userdata data = {};
	uint8_t buffer[1024];
	struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof(buffer));
	pw_init(&argc, &argv);

	char const* program_name = nob_shift_args(&argc, &argv);

	char const* source_file = argc > 0 ? nob_shift_args(&argc, &argv) : NULL;
	if (!source_file) {
		fprintf(stderr, "usage: %s <midifile>\n", basename(strdup(program_name)));
		exit_code = 1;
		goto cleanup;
	}

	data.midi_file = midi_file_open(source_file, "r", &data.midi_file_info);
	if (!data.midi_file) {
		fprintf(stderr, "[ERROR] Failed to read midi file %s: %s\n", source_file, strerror(errno));
		exit_code = 1;
		goto cleanup;
	}

	nob_log(NOB_INFO, "MIDI File: format %08x ntracks:%d div:%d",
			data.midi_file_info.format,
			data.midi_file_info.ntracks,
			data.midi_file_info.division);

	data.loop = pw_main_loop_new(NULL);
	if (data.loop == NULL) {
		fprintf(stderr, "[ERROR] Failed to create main loop: %s\n", strerror(errno));
		exit_code = 1;
		goto cleanup;
	}
	pw_loop_add_signal(pw_main_loop_get_loop(data.loop), SIGINT, do_quit, &data);
	pw_loop_add_signal(pw_main_loop_get_loop(data.loop), SIGTERM, do_quit, &data);

	data.context = pw_context_new(pw_main_loop_get_loop(data.loop), NULL, 0);
	if (data.context == NULL) {
		fprintf(stderr, "[ERROR] Failed to create context: %s\n", strerror(errno));
		exit_code = 1;
		goto cleanup;
	}

	// TODO: Ability to specify remote_name (if needed)
	char const* remote_name = "[" PW_DEFAULT_REMOTE "-manager," PW_DEFAULT_REMOTE "]";
	data.core = pw_context_connect(data.context, pw_properties_new(PW_KEY_REMOTE_NAME, remote_name, NULL), 0);
	if (data.core == NULL) {
		fprintf(stderr, "[ERROR] Failed to connect: %s\n", strerror(errno));
		exit_code = 1;
		goto cleanup;
	}

	static struct pw_core_events const core_events = {
		PW_VERSION_CORE_EVENTS,
		.done = on_core_done,
		.error = on_core_error,
	};
	pw_core_add_listener(data.core, &data.core_listener, &core_events, &data);


	static struct pw_registry_events const registry_events = {
		PW_VERSION_REGISTRY_EVENTS,
		.global = registry_event_global,
	};

	data.registry = pw_core_get_registry(data.core, PW_VERSION_REGISTRY, 0);
	pw_registry_add_listener(data.registry, &data.registry_listener, &registry_events, &data);

	// We request sync event which will hapen AFTER all previous regstered requests
	// This means that it will be the last event incoming and thus allow us to exit loop
	data.sync = pw_core_sync(data.core, PW_ID_CORE, data.sync);
	pw_main_loop_run(data.loop);

	for (size_t i = 0; i < data.found_ports.count; ++i) {
		struct port_info port = data.found_ports.items[i];
		char const *node_name = NULL;

		for (size_t j = 0; j < data.found_nodes.count; ++j) {
			struct node_info node = data.found_nodes.items[j];
			if (node.node_id == port.node_id) {
				node_name = node.node_name;
				break;
			}
		}
		assert(node_name != NULL);

		printf("%3s | %s:%s\n", port.port_serial, node_name, port.port_name);
	}

	// pw_stream_new(struct pw_core *core, const char *name, struct pw_properties *props);
	data.stream = pw_stream_new(data.core, "midi-src",
			pw_properties_new(
				PW_KEY_MEDIA_TYPE, "Midi",
				PW_KEY_MEDIA_CATEGORY, "Playback",
				PW_KEY_MEDIA_ROLE, "Music",
				NULL
			)
	);
	if (!data.stream) {
		nob_log(NOB_ERROR, "Failed to create stream: %s\n", strerror(errno));
		exit_code = 1;
		goto cleanup;
	}

	// pw_stream_add_listener(struct pw_stream *stream, struct spa_hook *listener, const struct pw_stream_events *events, void *data)

	static struct pw_stream_events const stream_events = {
		PW_VERSION_STREAM_EVENTS,
		.process = on_process,
		.state_changed = on_state_changed,
		.io_changed = on_io_changed,
		.drained = on_drained,
	};

	pw_stream_add_listener(data.stream, &data.stream_listener, &stream_events, &data);


	struct spa_pod *params[2];
	size_t n_params = 0;
	params[n_params++] = spa_pod_builder_add_object(&b,
			SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat,
			SPA_FORMAT_mediaType,		SPA_POD_Id(SPA_MEDIA_TYPE_application),
			SPA_FORMAT_mediaSubtype,	SPA_POD_Id(SPA_MEDIA_SUBTYPE_control));

	// pw_properties_set(props, PW_KEY_FORMAT_DSP, "8 bit raw midi");

	// pw_stream_connect(struct pw_stream *stream, enum spa_direction direction, uint32_t target_id, enum pw_stream_flags flags, const struct spa_pod **params, uint32_t n_params)

	int ret = pw_stream_connect(
		data.stream,
		PW_DIRECTION_OUTPUT,
		PW_ID_ANY,
		PW_STREAM_FLAG_MAP_BUFFERS,
		(const struct spa_pod **)params, n_params);

	if (ret < 0) {
		nob_log(NOB_ERROR, "Failed to connect to stream: %s\n", spa_strerror(ret));
		goto cleanup;
	}

	pw_main_loop_run(data.loop);

cleanup:
	if (data.stream) {
		spa_hook_remove(&data.stream_listener);
		pw_stream_disconnect(data.stream);
		pw_stream_destroy(data.stream);
	}
	if (data.core) {
		spa_hook_remove(&data.core_listener);
		pw_core_disconnect(data.core);
	}
	if (data.context) pw_context_destroy(data.context);
	if (data.loop)    pw_main_loop_destroy(data.loop);
	if (data.midi_file) midi_file_close(data.midi_file);
	pw_deinit();
	return exit_code;
}

