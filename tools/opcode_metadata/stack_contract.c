#define Py_BUILD_CORE
#define NEED_OPCODE_METADATA
#include "Python.h"
#include "pycore_opcode_metadata.h"
#include <stdio.h>
int main(void) {
    const int args[] = {0,1,2,3,4,5,7,8,15,16,31,255,256,65535};
    for (int op=0; op<512; op++) {
        for (unsigned int i=0; i<sizeof(args)/sizeof(args[0]); i++) {
            int popped = _PyOpcode_num_popped(op,args[i]);
            int pushed = _PyOpcode_num_pushed(op,args[i]);
            if (popped >= 0 && pushed >= 0) {
                printf("%d %d %d %d\n",op,args[i],popped,pushed);
            }
        }
    }
    return 0;
}
