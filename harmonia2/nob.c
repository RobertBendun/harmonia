#define NOB_IMPLEMENTATION
#include "nob.h"

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

int main(int argc, char **argv)
{
	NOB_GO_REBUILD_URSELF(argc, argv);

	char const* program_name = nob_shift_args(&argc, &argv);
	char const* subcommand = argc <= 0 ? "build" : nob_shift_args(&argc, &argv);


	bool build_always = strcmp(subcommand, "build-always") == 0;

	Nob_Cmd cmd = {};

	if (build_always || nob_needs_rebuild1("vendor/midifile/midifile.o", "vendor/midifile/midifile.c")) {
		nob_cc(&cmd);
		nob_cmd_append(&cmd, "-c");
		nob_cc_output(&cmd, "vendor/midifile/midifile.o");
		nob_cc_inputs(&cmd, "vendor/midifile/midifile.c");
		append_libpipewire(&cmd);
		if (!nob_cmd_run(&cmd)) return 1;
	}

	if (build_always || nob_needs_rebuild1("vendor/midifile/midievent.o", "vendor/midifile/midievent.c")) {
		nob_cc(&cmd);
		nob_cmd_append(&cmd, "-c");
		nob_cc_output(&cmd, "vendor/midifile/midievent.o");
		nob_cc_inputs(&cmd, "vendor/midifile/midievent.c");
		append_libpipewire(&cmd);
		if (!nob_cmd_run(&cmd)) return 1;
	}

	char const* pipewire_midi_demo_inputs[] = {
		"pipewire_midi_demo.c",
		"vendor/midifile/midifile.o",
		"vendor/midifile/midievent.o",
	};
	if (build_always || nob_needs_rebuild("pipewire_midi_demo", pipewire_midi_demo_inputs, NOB_ARRAY_LEN(pipewire_midi_demo_inputs))) {
		nob_cc(&cmd);
		nob_cc_flags(&cmd);
		nob_cc_output(&cmd, "pipewire_midi_demo");
		nob_cmd_append(&cmd, "-Ivendor/midifile/");
		nob_da_append_many(&cmd, pipewire_midi_demo_inputs, NOB_ARRAY_LEN(pipewire_midi_demo_inputs));
		append_libpipewire(&cmd);
		if (!nob_cmd_run(&cmd)) return 1;
	}


	if (strcmp(subcommand, "run") == 0) {
		nob_cmd_append(&cmd, "./pipewire_midi_demo");
		nob_da_append_many(&cmd, argv, argc);
		if (!nob_cmd_run(&cmd)) return 1;
	}

	return 0;
}
