/* SPDX-License-Identifier: GPL-3.0-or-later
 * Native exec-only launcher: no shell, PATH search or Python environment input.
 */
#include <mach-o/dyld.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    char executable[PATH_MAX], directory[PATH_MAX], python[PATH_MAX], script[PATH_MAX];
    uint32_t size = sizeof executable;
    if (_NSGetExecutablePath(executable, &size) != 0 || !realpath(executable, directory))
        goto unavailable;
    char *slash = strrchr(directory, '/');
    if (!slash) goto unavailable;
    *slash = '\0';
    if ((size_t)snprintf(python, sizeof python, "%s/../Resources/HelperRuntime/Python/bin/python3.14", directory) >= sizeof python ||
        (size_t)snprintf(script, sizeof script, "%s/../Resources/HelperRuntime/yt_dlp_bootstrap.py", directory) >= sizeof script)
        goto unavailable;
    char **arguments = calloc((size_t)argc + 5, sizeof *arguments);
    if (!arguments) goto unavailable;
    arguments[0] = python;
    arguments[1] = "-I";
    arguments[2] = "-S";
    arguments[3] = "-B";
    arguments[4] = script;
    for (int i = 1; i < argc; i++) arguments[i + 4] = argv[i];
    execv(python, arguments);
    free(arguments);
 unavailable:
    fputs("The bundled Python helper is unavailable.\n", stderr);
    return 127;
}
