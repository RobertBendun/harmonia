#include <assert.h>
#include <lauxlib.h>
#include <lua.h>
#include <rtmidi/rtmidi_c.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <math.h>
#include <stdbool.h>

typedef struct
{
	float prev, now, note_remaining_time[128], sleep;
} state;

static void init(state *s);
static void forward(state *s, RtMidiOutPtr rtmidi);
static bool execute(state *s, RtMidiOutPtr rtmidi, bool coroutine_ended);

static void note_on(RtMidiOutPtr device, unsigned note);
static void note_off(RtMidiOutPtr device, unsigned note);
static float current_time();
static void sleep_for(float f);

static int l_bind_block(lua_State *L)
{
	char const *path, *action;
	lua_State *co;
	int note, nres;
	float duration;
	RtMidiOutPtr rtmidi;
	state s = {};

	/* Ensure that user provided correct arguments */
	path = luaL_checkstring(L, 1);
	luaL_checktype(L, 2, LUA_TFUNCTION);

	/* Create coroutine and move "block" to it */
	co = lua_newthread(L);
	lua_pushvalue(L, 2);
	lua_xmove(L, co, 1);

	/* Create MIDI context */
	rtmidi = rtmidi_out_create_default();
	rtmidi_open_port(rtmidi, 0, "Harmonia test");

	init(&s);
	for (;;) {
		/* Execute code in coroutine until coroutine ends */
		if (co) {
			switch (lua_resume(co, L, 0, &nres)) {
				break; case LUA_OK:
					co = NULL; /* TODO: Check how to properly clean coroutines */

				break; case LUA_YIELD:
					action = luaL_checkstring(co, 1);
					if (strcmp(action, "play") == 0) {
						note = luaL_checkinteger(co, 2);
						duration = luaL_checknumber(co, 3);

						if (s.note_remaining_time[note] <= 0)
							note_on(rtmidi, note);

						if (s.note_remaining_time[note] < duration)
							s.note_remaining_time[note] = duration;
					} else if (strcmp(action, "sleep") == 0) {
						s.sleep = luaL_checknumber(co, 2);
					} else {
						rtmidi_close_port(rtmidi);
						lua_pushliteral(co, "failed to recognize action");
						lua_error(co);
					}

				break; default:
					assert(0 && "not implemented yet");
			}
		}

		if (!execute(&s, rtmidi, !co))
			break;
	}

	rtmidi_close_port(rtmidi);

	return 1;
}


static const struct luaL_Reg harmonia[] = {
	{"bind_block", l_bind_block},
	{NULL, NULL},
};

int luaopen_harmonia(lua_State *L)
{
	lua_newtable(L);
	luaL_newlib(L, harmonia);
	return 1;
}

static void note_on(RtMidiOutPtr device, unsigned note)
{
	fprintf(stderr, "executor: midi: note on: %u\n", note);
	unsigned char message[3] = { 0b10010000, note, 100 };
	rtmidi_out_send_message(device, message, sizeof(message));
}

static void note_off(RtMidiOutPtr device, unsigned note)
{
	fprintf(stderr, "executor: midi: note off: %u\n", note);
	unsigned char message[3] = { 0b10000000, note, 0 };
	rtmidi_out_send_message(device, message, sizeof(message));
}

static float current_time()
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (float)ts.tv_sec + ts.tv_nsec/1000000000.0;
}

static void sleep_for(float f)
{
	if (f < 0)
		return;

	fprintf(stderr, "executor: sleep for: %f\n", f);
	nanosleep(&(struct timespec) {
			.tv_sec = floorf(f),
			.tv_nsec = (f - floorf(f)) * 1000000000,
	}, NULL);
}

static void init(state *s)
{
	s->now = current_time();
}

static void forward(state *s, RtMidiOutPtr rtmidi)
{
	size_t i;
	s->prev = s->now;
	s->now = current_time();

	/* Update remaining note times based on how much time has passed */
	for (i = 0; i < sizeof(s->note_remaining_time) / sizeof(*s->note_remaining_time); ++i) {
		if (s->note_remaining_time[i] > 0.0) {
			s->note_remaining_time[i] -= s->now - s->prev;
			if (s->note_remaining_time[i] <= 0.0) {
				note_off(rtmidi, i);
			}
		}
	}

	/* Update sleep based on how much time has passed */
	if (s->sleep > 0) {
		s->sleep -= s->now - s->prev;
	}
}

static bool execute(state *s, RtMidiOutPtr rtmidi, bool coroutine_ended)
{
	float min_wait_time;
	size_t i;

	forward(s, rtmidi);

	/* If user defined sleep that didn't expired yet or coroutine ended then we can sleep for some time */
	if (s->sleep > 0 || coroutine_ended) {
sleep_again:
		min_wait_time = s->sleep > 0 ? s->sleep : INFINITY;
		for (i = 0; i < sizeof(s->note_remaining_time) / sizeof(*s->note_remaining_time); ++i) {
			if (s->note_remaining_time[i] > 0 && s->note_remaining_time[i] < min_wait_time) {
				min_wait_time = s->note_remaining_time[i];
			}
		}
		if (min_wait_time != INFINITY) {
			sleep_for(s->prev + min_wait_time - s->now);
			forward(s, rtmidi);
			if (s->sleep > 0) {
				/* Ensure that if we slept less then expected, we still sleep the remaining part */
				goto sleep_again;
			}
		} else if (coroutine_ended) {
			/* If coroutine ended and we don't have to sleep then we ended playing given block */
			return false;
		}
	}

	return true;
}
