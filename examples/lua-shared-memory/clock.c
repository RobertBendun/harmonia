#include <time.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <math.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <signal.h>
#include <fcntl.h>

static int fd;
char const* path = "/harmonia-block";

void clean(int)
{
	shm_unlink(path);
	exit(1);
}

int main()
{
	signal(SIGINT, clean);

	if ((fd = shm_open(path, O_RDWR|O_CREAT, 0600)) < 0) {
		perror("shm_open");
		return 1;
	}

	ftruncate(fd, sizeof(double));

	double* p = mmap(NULL, sizeof(double), PROT_READ|PROT_WRITE, MAP_SHARED, fd, 0);

	for (;;) {
		struct timespec ts;
		clock_gettime(CLOCK_MONOTONIC, &ts);
		*p = (double)ts.tv_sec + ts.tv_nsec/1000000000.0;

		float f = 0.005;
		nanosleep(&(struct timespec) {
				.tv_sec = floor(f),
				.tv_nsec = (f - floor(f)) * 1000000000,
		}, NULL);
	}
}
