// TODOs:
// - [x] Establish per interface outgoing sockets
// - [x] Establish listening socket
// - [x] Establish graceful exit
// - [ ] Time service that would wake us up every n <insert unit of time> to broadcast our state
// - [ ] Broadcast state using per interface sockets, respecting their ready state (some kind of broadcast flag?)
// - [ ] Measurement service
// - [ ] Almost all file descriptors should be close on exec

#include <errno.h>
#include <inttypes.h>
#include <stddef.h>

#define NOB_IMPLEMENTATION
#include "vendor/nob.h"

#include <arpa/inet.h>
#include <ifaddrs.h>
#include <linux/if.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/epoll.h>
#include <sys/signalfd.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

// https://tldp.org/HOWTO/text/Multicast-HOWTO

#define MULTICAST_ADDRESS "224.68.66.72"
#define MULTICAST_PORT 22026

struct per_interface_socket
{
	int fd;
	bool ready;
};

struct per_interface_sockets
{
	struct per_interface_socket *items;
	size_t count, capacity;
};

struct close_pool
{
	int *items;
	size_t count, capacity;
};

struct [[gnu::packed]] broadcast_message
{
	uint64_t host_time_us;
};

struct [[gnu::packed]] broadcast_message_raw
{
	char data[sizeof(struct broadcast_message)];
};

static_assert(sizeof(struct broadcast_message) == sizeof(struct broadcast_message_raw),
	"Broadcast message should have the same size in parsed and raw form");

struct app_state
{
	int epoll_fd;
	bool quit;
};

struct service;
typedef void(*generic_service_t)(struct app_state *app_state, struct service *self);

struct service
{
	generic_service_t service;
	int fd;
};

#define ENSURE_VALID_SERVICE(T) \
	static_assert(offsetof(struct service, service) == offsetof(T, service), "Service field should have the same offset"); \
	static_assert(sizeof(((struct service*)0)->service) == sizeof(((T*)0)->service), "Service field should have the same size"); \
	static_assert(offsetof(struct service, fd) == offsetof(T, fd), "Fd field should have the same offset"); \
	static_assert(sizeof(((struct service*)0)->fd) == sizeof(((T*)0)->fd), "Fd field should have the same size");

int register_service(struct app_state *app_state, struct service *pinned_service, uint32_t events)
{
	assert(app_state);
	assert(pinned_service);

	struct epoll_event ev = {};
	ev.events = events;
	ev.data.ptr = pinned_service;
	return epoll_ctl(app_state->epoll_fd, EPOLL_CTL_ADD, pinned_service->fd, &ev);
}

struct state_broadcast_service
{
	void (*service)(struct app_state *app_state, struct state_broadcast_service *self);
	int fd;
	bool ready;
};
ENSURE_VALID_SERVICE(struct state_broadcast_service);

void service_state_broadcast(struct app_state *app_state, struct state_broadcast_service *self)
{
	(void)app_state;
	self->ready = true;
}

void service_signal(struct app_state *app_state, struct service *self)
{

	struct signalfd_siginfo siginfo = {};
	int r = read(self->fd, &siginfo, sizeof(siginfo));
	if (r != sizeof(siginfo)) {
		nob_log(NOB_ERROR, "Failed to read signal information properly: %s", strerror(errno));
		return;
	}
	if (siginfo.ssi_signo == SIGINT || siginfo.ssi_signo == SIGQUIT) {
		nob_log(NOB_INFO, "SIGINT/SIGQUIT received, exiting gracefully");
		app_state->quit = true;
	}
}

void service_nop(struct app_state *app_state, struct service *self)
{
	(void)app_state;
	(void)self;
}

uint64_t get_host_time_us()
{
	// Note that 2**64 - 1 microseconds is almost 600 000 years so we will be fine
	// Also we are using CLOCK_BOOTTIME and not CLOCK_MONOTONIC for the case when laptop would suspend mid performance
	struct timespec ts;
	if (clock_gettime(CLOCK_BOOTTIME, &ts) < 0) {
		nob_log(NOB_ERROR, "Couldn't receive the CLOCK_BOOTTIME, this should _never_ happen: %s", strerror(errno));
		exit(1);
	}
	uint64_t const seconds_to_microseconds_num = 1000000;
	uint64_t const nano_to_microseconds_den = 1000;
	return ts.tv_sec * seconds_to_microseconds_num + ts.tv_nsec / nano_to_microseconds_den;
}

int main()
{
	struct app_state app_state = {};

	struct close_pool close_pool = {};
	int return_code = 1;

	struct
	{
		struct state_broadcast_service **items;
		size_t count, capacity;
	} state_broadcast_services = {};

	app_state.epoll_fd = epoll_create1(0);
	if (app_state.epoll_fd < 0) {
		nob_log(NOB_ERROR, "failed to initialize epoll: %s", strerror(errno));
		goto exit;
	}
	nob_da_append(&close_pool, app_state.epoll_fd);

	struct ifaddrs *interfaces_list;
	if (getifaddrs(&interfaces_list) < 0) {
		nob_log(NOB_ERROR, "failed to get list of network interfaces: %s", strerror(errno));
		return 1;
	}

	for (struct ifaddrs const* iface = interfaces_list; iface; iface = iface->ifa_next) {
		char interface[INET_ADDRSTRLEN] = {};

		switch (iface->ifa_addr->sa_family) {
		case AF_INET:
			{
				int r = getnameinfo(iface->ifa_addr,
						sizeof(struct sockaddr_in),
						interface, NI_MAXHOST,
						NULL, 0, NI_NUMERICHOST);
				assert(r == 0);

				int sockfd = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
				if (sockfd < 0) {
					nob_log(NOB_ERROR, "Failed to establish socket on interface %s: %s", iface->ifa_name, strerror(errno));
					continue;
				}
				nob_da_append(&close_pool, sockfd);

				//r = setsockopt(sockfd, IPPROTO_IP, IP_MULTICAST_IF, &iface_addr, sizeof(iface_addr));
				//if (r < 0) {
				//	nob_log(NOB_ERROR, "Failed to establish multicast on interface %s: %s", iface->ifa_name, strerror(errno));
				//	close(sockfd);
				//	continue;
				//}

				struct ip_mreq multicast_group = {};
				if (!inet_aton(MULTICAST_ADDRESS, &multicast_group.imr_multiaddr)) {
					nob_log(NOB_ERROR, "Failed to parse specified multicast address");
					goto exit;
				}
				if (!inet_aton(interface, &multicast_group.imr_interface)) {
					nob_log(NOB_ERROR, "Failed to parse specified interface address");
					goto exit;
				}
				r = setsockopt(sockfd, IPPROTO_IP, IP_ADD_MEMBERSHIP, &multicast_group, sizeof(multicast_group));
				if (r < 0) {
					nob_log(NOB_ERROR, "Failed to join multicast group on interface %s: %s", iface->ifa_name, strerror(errno));
					continue;
				}

				// Node that for file descriptors for writing we want the edge triggered flavour so we are not constantly
				// spammed when we don't want to send any data (and we would probably don't want to send any data quite often)
				struct state_broadcast_service *sb = calloc(1, sizeof(struct state_broadcast_service));
				sb->ready = true;
				sb->fd = sockfd;
				sb->service = service_state_broadcast;
				if (register_service(&app_state, (struct service*)sb, EPOLLOUT|EPOLLET) < 0) {
					nob_log(NOB_ERROR, "Failed to add socket of interface %s to epoll group: %s", iface->ifa_name, strerror(errno));
					continue;
				}

				nob_da_append(&state_broadcast_services, sb);
				nob_log(NOB_INFO, "Established socket on %s (%s)", iface->ifa_name, interface);
			}
			break;
		}
	}
	freeifaddrs(interfaces_list);

	int listening_sockfd = socket(AF_INET, SOCK_DGRAM|SOCK_NONBLOCK, 0);
	if (listening_sockfd < 0) {
		nob_log(NOB_ERROR, "Failed to establish receiving socket: %s", strerror(errno));
		goto exit;
	}
	nob_da_append(&close_pool, listening_sockfd);

	struct ip_mreq multicast_group = {};
	if (!inet_aton(MULTICAST_ADDRESS, &multicast_group.imr_multiaddr)) {
		nob_log(NOB_ERROR, "Failed to parse specified multicast address");
		goto exit;
	}
	multicast_group.imr_interface.s_addr = INADDR_ANY;
	int r = setsockopt(listening_sockfd, IPPROTO_IP, IP_ADD_MEMBERSHIP, &multicast_group, sizeof(multicast_group));
	if (r < 0) {
		nob_log(NOB_ERROR, "Failed to join multicast group on receiving socket: %s", strerror(errno));
		goto exit;
	}

	struct sockaddr_in listening_address = {};
	listening_address.sin_family = AF_INET;
	listening_address.sin_addr.s_addr = INADDR_ANY;
	listening_address.sin_port = htons(MULTICAST_PORT);
	r = bind(listening_sockfd, (struct sockaddr const*)&listening_address, sizeof(listening_address));
	if (r < 0) {
		nob_log(NOB_ERROR, "Failed to bind receiving socket on port %d: %s", MULTICAST_PORT, strerror(errno));
		goto exit;
	}

	struct service listening_service = {};
	listening_service.fd = listening_sockfd;
	listening_service.service = service_nop;
	if (register_service(&app_state, &listening_service, EPOLLIN) < 0) {
		nob_log(NOB_ERROR, "Failed to epoll receiving socket: %s", strerror(errno));
		return 1;
	}

	nob_log(NOB_INFO, "Listening on 0.0.0.0:%d", MULTICAST_PORT);

	sigset_t signals_mask = {};

	sigemptyset(&signals_mask);
	sigaddset(&signals_mask, SIGINT);
	sigaddset(&signals_mask, SIGQUIT);

	// Ensure that masked signals wont be handled by the usual means
	if (sigprocmask(SIG_BLOCK, &signals_mask, NULL) < 0) {
		nob_log(NOB_ERROR, "Failed to block signals: %s", strerror(errno));
		close(listening_sockfd);
		return 1;
	}

	int signals_fd = signalfd(-1, &signals_mask, SFD_NONBLOCK);
	if (signals_fd < 0) {
		nob_log(NOB_ERROR, "Failed to establish signals file descriptor: %s", strerror(errno));
		close(listening_sockfd);
		return 1;
	}
	nob_da_append(&close_pool, signals_fd);

	struct service signal_service = { .service = service_signal, .fd = signals_fd };
	if (register_service(&app_state, &signal_service, EPOLLIN) < 0) {
		nob_log(NOB_ERROR, "Failed to epoll signals: %s", strerror(errno));
		return 1;
	}

	// FIXME: Should be proportional to the number of registered descriptors.
	//        We can get that by number of calls to register_service
	struct epoll_event events[16];

	while (!app_state.quit) {
		r = epoll_wait(app_state.epoll_fd, events, NOB_ARRAY_LEN(events), -1);
		if (r < 0) {
			// FIXME: Are there errors that we should expect and handle gracefully?
			nob_log(NOB_ERROR, "Failed to epoll_wait: %s", strerror(errno));
			return 1;
		}
		for (int i = 0, n = r; i < n; ++i) {
			struct epoll_event ev = events[i];
			((struct service*)ev.data.ptr)->service(&app_state, ev.data.ptr);
		}
	}

	return_code = 0;
exit:
	for (size_t i = 0; i < close_pool.count; ++i) {
		close(close_pool.items[i]);
	}
	nob_da_free(close_pool);

	for (size_t i = 0; i < state_broadcast_services.count; ++i) {
		free(state_broadcast_services.items[i]);
	}
	nob_da_free(state_broadcast_services);

	return return_code;
}
