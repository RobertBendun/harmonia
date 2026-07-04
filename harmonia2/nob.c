#define NOB_IMPLEMENTATION
#include "vendor/nob.h"

#define FLAG_IMPLEMENTATION
#include "vendor/flag.h"

static bool build_always = false;

void append_libpipewire(Nob_Cmd *cmd)
{
	// Result of execution `pkg-config -cflags -libs libpipewire-0.3` on my machine
	nob_cmd_append(cmd,
			"-I/usr/include/pipewire-0.3",
			"-I/usr/include/spa-0.2",
			"-D_REENTRANT",
			"-fno-strict-aliasing",
			"-fno-strict-overflow",
			"-lpipewire-0.3",
			"-lm");
}

bool build_pipewire_demo(Nob_Cmd *cmd)
{
	if (build_always || nob_needs_rebuild1("vendor/midifile/midifile.o", "vendor/midifile/midifile.c")) {
		nob_cc(cmd);
		nob_cmd_append(cmd, "-c");
		nob_cc_output(cmd, "vendor/midifile/midifile.o");
		nob_cc_inputs(cmd, "vendor/midifile/midifile.c");
		append_libpipewire(cmd);
		if (!nob_cmd_run(cmd)) return false;
	}

	if (build_always || nob_needs_rebuild1("vendor/midifile/midievent.o", "vendor/midifile/midievent.c")) {
		nob_cc(cmd);
		nob_cmd_append(cmd, "-c");
		nob_cc_output(cmd, "vendor/midifile/midievent.o");
		nob_cc_inputs(cmd, "vendor/midifile/midievent.c");
		append_libpipewire(cmd);
		if (!nob_cmd_run(cmd)) return false;
	}

	char const* pipewire_midi_demo_inputs[] = {
		"pipewire_midi_demo.c",
		"vendor/midifile/midifile.o",
		"vendor/midifile/midievent.o",
	};
	if (build_always || nob_needs_rebuild("pipewire_midi_demo", pipewire_midi_demo_inputs, NOB_ARRAY_LEN(pipewire_midi_demo_inputs))) {
		nob_cc(cmd);
		nob_cc_flags(cmd);
		nob_cc_output(cmd, "pipewire_midi_demo");
		nob_cmd_append(cmd, "-Ivendor/midifile/");
		nob_da_append_many(cmd, pipewire_midi_demo_inputs, NOB_ARRAY_LEN(pipewire_midi_demo_inputs));
		append_libpipewire(cmd);
		if (!nob_cmd_run(cmd)) return false;
	}

	return true;
}

bool build_linux_network_demo(Nob_Cmd *cmd)
{
	if (build_always || nob_needs_rebuild1("linux_network_demo", "linux_network_demo.c")) {
		nob_cc(cmd);
		nob_cc_output(cmd, "linux_network_demo");
		nob_cc_inputs(cmd, "linux_network_demo.c");
		if (!nob_cmd_run(cmd)) return false;
	}
	return true;
}

int main(int argc, char **argv)
{
	NOB_GO_REBUILD_URSELF(argc, argv);

	bool help = false;
	bool run_pipewire = false;
	bool run_networking = false;

	flag_bool_var(&build_always, "B", false, "Rebuild all");
	flag_bool_var(&help, "help", false, "Print this help message");
	flag_bool_var(&run_pipewire, "run-pw", false, "Run pipewire example. Mutually exclusive with other run flags");
	flag_bool_var(&run_networking, "run-net", false, "Run networking example. Mutually exclusive with other run flags");

	if (!flag_parse(argc, argv)) {
		flag_print_error(stderr);
		return 1;
	}

	if (help) {
		flag_print_options(stdout);
		return 0;
	}

	Nob_Cmd cmd = {};

	if (!build_pipewire_demo(&cmd)) return 1;
	if (!build_linux_network_demo(&cmd)) return 1;

	if (run_pipewire) {
		nob_cmd_append(&cmd, "./pipewire_midi_demo");
		nob_da_append_many(&cmd, flag_rest_argv(), flag_rest_argc());
		if (!nob_cmd_run(&cmd)) return 1;
		return 0;
	}

	if (run_networking) {
		nob_cmd_append(&cmd, "./linux_network_demo");
		nob_da_append_many(&cmd, flag_rest_argv(), flag_rest_argc());
		if (!nob_cmd_run(&cmd)) return 1;
		return 0;
	}

	return 0;
}
