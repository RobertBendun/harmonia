all: harmonia.so clock

harmonia.so: harmonia.c
	gcc $< -shared -o $@ -fPIC  -llua -lrtmidi -Wall -Wextra

LDLIBS=-lm -lrt
