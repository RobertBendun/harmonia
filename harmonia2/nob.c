#define NOB_IMPLEMENTATION
#include "nob.h"

int main(int argc, char **argv)
{
	NOB_GO_REBUILD_URSELF(argc, argv);

	char const* program_name = nob_shift_args(&argc, &argv);
	char const* subcommand = argc <= 0 ? "build" : nob_shift_args(&argc, &argv);

	Nob_Cmd cmd = {};

	if (nob_needs_rebuild1("pipewire_midi_demo", "pipewire_midi_demo.c")) {
		nob_cc(&cmd);
		nob_cc_flags(&cmd);
		nob_cc_output(&cmd, "pipewire_midi_demo");
		nob_cc_inputs(&cmd, "pipewire_midi_demo.c");


		// Result of execution `pkg-config -cflags -libs libpipewire-0.3` on my machine
		nob_cmd_append(&cmd,
				"-I/usr/include/pipewire-0.3",
				"-I/usr/include/spa-0.2",
				"-D_REENTRANT",
				"-fno-strict-aliasing",
				"-fno-strict-overflow",
				"-lpipewire-0.3");

		if (!nob_cmd_run(&cmd)) return 1;
	}


	if (strcmp(subcommand, "run") == 0) {
		nob_cmd_append(&cmd, "./pipewire_midi_demo");
		if (!nob_cmd_run(&cmd)) return 1;
	}

	return 0;
}
