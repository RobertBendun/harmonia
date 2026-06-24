#include <errno.h>
#include <pipewire/pipewire.h>
#include <stdlib.h>
#include <string.h>
#include <spa/utils/result.h>

#define NOB_IMPLEMENTATION
#include "nob.h"

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

	struct spa_hook core_listener, registry_listener;

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


int main(int argc, char **argv)
{
	int exit_code = 0;
	struct userdata data = {};
	pw_init(&argc, &argv);


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

cleanup:
	if (data.core)    pw_core_disconnect(data.core);
	if (data.context) pw_context_destroy(data.context);
	if (data.loop)    pw_main_loop_destroy(data.loop);
	pw_deinit();
	return exit_code;
}

