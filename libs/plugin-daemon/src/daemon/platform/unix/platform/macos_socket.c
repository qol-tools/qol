#include <libproc.h>
#include <sys/proc_info.h>
#include <sys/socket.h>
#include <unistd.h>

int qol_daemon_socket_is_listening(int fd) {
    struct socket_fdinfo info = {0};
    int size = (int)sizeof(info);
    int bytes = proc_pidfdinfo(getpid(), fd, PROC_PIDFDSOCKETINFO,
                              &info, size);
    if (bytes != size) {
        return -1;
    }
    return (info.psi.soi_options & SO_ACCEPTCONN) != 0;
}
