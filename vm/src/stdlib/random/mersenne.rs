/*
   A C-program for MT19937, with initialization improved 2002/1/26.
   Coded by Takuji Nishimura and Makoto Matsumoto.

   Before using, initialize the state by using init_genrand(seed)
   or init_by_array(init_key, key_length).

   Copyright (C) 1997 - 2002, Makoto Matsumoto and Takuji Nishimura,
   All rights reserved.

   Redistribution and use in source and binary forms, with or without
   modification, are permitted provided that the following conditions
   are met:

     1. Redistributions of source code must retain the above copyright
        notice, this list of conditions and the following disclaimer.

     2. Redistributions in binary form must reproduce the above copyright
        notice, this list of conditions and the following disclaimer in the
        documentation and/or other materials provided with the distribution.

     3. The names of its contributors may not be used to endorse or promote
        products derived from this software without specific prior written
        permission.

   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
   "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
   LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
   A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR
   CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
   EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
   PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
   PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
   LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
   NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
   SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


   Any feedback is very welcome.
   http://www.math.sci.hiroshima-u.ac.jp/~m-mat/MT/emt.html
   email: m-mat @ math.sci.hiroshima-u.ac.jp (remove space)
*/

// this was translated from c; all rights go to copyright holders listed above
// https://gist.github.com/coolreader18/b56d510f1b0551d2954d74ad289f7d2e

/* Period parameters */
const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908b0dfu32; /* constant vector a */
const UPPER_MASK: u32 = 0x80000000u32; /* most significant w-r bits */
const LOWER_MASK: u32 = 0x7fffffffu32; /* least significant r bits */

pub struct MT19937 {
    mt: [u32; N], /* the array for the state vector  */
    mti: usize,   /* mti==N+1 means mt[N] is not initialized */
}
impl Default for MT19937 {
    fn default() -> Self {
        MT19937 {
            mt: [0; N],
            mti: N + 1,
        }
    }
}
impl std::fmt::Debug for MT19937 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.pad("MT19937")
    }
}

impl MT19937 {
    pub fn new_with_slice_seed(init_key: &[u32]) -> Self {
        let mut state = Self::default();
        state.seed_slice(init_key);
        state
    }

    /* initializes self.mt[N] with a seed */
    fn seed(&mut self, s: u32) {
        self.mt[0] = s & 0xffffffffu32;
        self.mti = 1;
        while self.mti < N {
            self.mt[self.mti] = 1812433253u32
                .wrapping_mul(self.mt[self.mti - 1] ^ (self.mt[self.mti - 1] >> 30))
                + self.mti as u32;
            /* See Knuth TAOCP Vol2. 3rd Ed. P.106 for multiplier. */
            /* In the previous versions, MSBs of the seed affect   */
            /* only MSBs of the array self.mt[].                        */
            /* 2002/01/09 modified by Makoto Matsumoto             */
            self.mt[self.mti] &= 0xffffffffu32;
            /* for >32 bit machines */
            self.mti += 1;
        }
    }

    /* initialize by an array with array-length */
    /* init_key is the array for initializing keys */
    /* key_length is its length */
    /* slight change for C++, 2004/2/26 */
    pub fn seed_slice(&mut self, init_key: &[u32]) {
        let mut i;
        let mut j;
        let mut k;
        self.seed(19650218);
        i = 1;
        j = 0;
        k = if N > init_key.len() {
            N
        } else {
            init_key.len()
        };
        while k != 0 {
            self.mt[i] = (self.mt[i]
                ^ ((self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1664525u32)))
                + init_key[j]
                + j as u32; /* non linear */
            self.mt[i] &= 0xffffffffu32; /* for WORDSIZE > 32 machines */
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= init_key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k != 0 {
            self.mt[i] = (self.mt[i]
                ^ ((self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1566083941u32)))
                - i as u32; /* non linear */
            self.mt[i] &= 0xffffffffu32; /* for WORDSIZE > 32 machines */
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            k -= 1;
        }

        self.mt[0] = 0x80000000u32; /* MSB is 1; assuring non-zero initial array */
    }

    /* generates a random number on [0,0xffffffff]-interval */
    fn gen_u32(&mut self) -> u32 {
        let mut y: u32;
        let mag01 = |x| if (x & 0x1) == 1 { MATRIX_A } else { 0 };
        /* mag01[x] = x * MATRIX_A  for x=0,1 */

        if self.mti >= N {
            /* generate N words at one time */

            if self.mti == N + 1
            /* if seed() has not been called, */
            {
                self.seed(5489u32);
            } /* a default initial seed is used */

            for kk in 0..N - M {
                y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
                self.mt[kk] = self.mt[kk + M] ^ (y >> 1) ^ mag01(y);
            }
            for kk in N - M..N - 1 {
                y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
                self.mt[kk] = self.mt[kk.wrapping_add(M.wrapping_sub(N))] ^ (y >> 1) ^ mag01(y);
            }
            y = (self.mt[N - 1] & UPPER_MASK) | (self.mt[0] & LOWER_MASK);
            self.mt[N - 1] = self.mt[M - 1] ^ (y >> 1) ^ mag01(y);

            self.mti = 0;
        }

        y = self.mt[self.mti];
        self.mti += 1;

        /* Tempering */
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680u32;
        y ^= (y << 15) & 0xefc60000u32;
        y ^= y >> 18;

        y
    }
}

impl rand::RngCore for MT19937 {
    fn next_u32(&mut self) -> u32 {
        self.gen_u32()
    }
    fn next_u64(&mut self) -> u64 {
        rand_core::impls::next_u64_via_u32(self)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        rand_core::impls::fill_bytes_via_next(self, dest)
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        Ok(self.fill_bytes(dest))
    }
}
