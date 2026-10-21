// luacrun — execute a precompiled Luau bytecode chunk (any LBC version the VM
// accepts), exactly as a game client would after loadstring().
//
//   luacrun <file.luac> [args...]
//
// Exit status mirrors the chunk's own error status so the round-trip harness
// can compare process-level behaviour, not just stdout.
#include "lua.h"
#include "lualib.h"


#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

static std::vector<char> readfile(const char* path)
{
    std::vector<char> out;
    FILE* f = fopen(path, "rb");
    if (!f)
    {
        fprintf(stderr, "luacrun: cannot open %s\n", path);
        exit(2);
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n > 0)
    {
        out.resize((size_t)n);
        if (fread(out.data(), 1, (size_t)n, f) != (size_t)n)
        {
            fprintf(stderr, "luacrun: short read on %s\n", path);
            exit(2);
        }
    }
    fclose(f);
    return out;
}

int main(int argc, char** argv)
{
    if (argc < 2)
    {
        fprintf(stderr, "usage: luacrun <file.luac> [args...]\n");
        return 2;
    }

    std::vector<char> data = readfile(argv[1]);

    lua_State* L = luaL_newstate();
    if (!L)
    {
        fprintf(stderr, "luacrun: cannot create state\n");
        return 2;
    }
    luaL_openlibs(L);

    if (luau_load(L, argv[1], data.data(), data.size(), 0) != 0)
    {
        const char* err = lua_tostring(L, -1);
        fprintf(stderr, "luacrun: load error: %s\n", err ? err : "(unknown)");
        lua_close(L);
        return 1;
    }

    // Push script arguments so the chunk sees them as `...`
    int nargs = argc - 2;
    if (nargs > 0)
    {
        lua_checkstack(L, nargs);
        for (int i = 0; i < nargs; ++i)
            lua_pushstring(L, argv[2 + i]);
    }

    int status = lua_pcall(L, nargs, 0, 0);
    if (status != 0)
    {
        const char* err = lua_tostring(L, -1);
        fprintf(stderr, "%s\n", err ? err : "(runtime error)");
        lua_close(L);
        return status == LUA_ERRMEM ? 3 : 1;
    }

    lua_close(L);
    return 0;
}
