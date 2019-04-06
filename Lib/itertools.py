import operator

class accumulate:
    def __init__(self, iterable, func = operator.add):
        self.it = iter(iterable)
        self._total = None
        self.func = func
        
    def __iter__(self):
        return self

    def __next__(self):
        if not self._total:
            self._total = next(self.it)
            return self._total
        else:
            element = next(self.it)
            try:
                self._total = self.func(self._total, element)
            except:
                raise TypeError("unsupported operand type")
            return self._total
                
## Adapted from:
## https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-34
class chain:
    def __init__(self, *iterables):
        self._iterables_iter = iter(map(iter, iterables))
        # little trick for the first chain.__next__() call
        self._cur_iterable_iter = iter([])

    def __iter__(self):
        return self
    
    def __next__(self):
        while True:
            try:
                return next(self._cur_iterable_iter)
            except StopIteration:
                self._cur_iterable_iter = next(self._iterables_iter)
    
    @classmethod
    def from_iterable(cls, iterable):
        for it in iterable:
            for element in it:
                yield element
                
class combinations:
    def __init__(self, iterable, r):
        self.pool = tuple(iterable)
        self.n = len(self.pool)
        self.r = r
        self.indices = list(range(self.r))
        self.zero = False

    def __iter__(self):
        return self

    def __next__(self):
        if self.r > self.n:
            raise StopIteration
        if not self.zero:
            self.zero = True
            return tuple(self.pool[i] for i in self.indices)
        else:
            try:
                for i in reversed(range(self.r)):
                    if self.indices[i] != i + self.n - self.r:
                        break
                self.indices[i] += 1
                for j in range(i+1, self.r):
                    self.indices[j] = self.indices[j-1] + 1
                return tuple(self.pool[i] for i in self.indices)
            except:
                raise StopIteration
                
class combinations_with_replacement:
    def __init__(self, iterable, r):
        self.pool = tuple(iterable)
        self.n = len(self.pool)
        self.r = r
        self.indices = [0] * self.r
        self.zero = False
        
    def __iter__(self):
        return self
    
    def __next__(self):
        if not self.n and self.r:
            raise StopIteration
        if not self.zero:
            self.zero = True
            return tuple(self.pool[i] for i in self.indices)
        else:
            try:
                for i in reversed(range(self.r)):
                    if self.indices[i] != self.n - 1:
                        break
                self.indices[i:] = [self.indices[i] + 1] * (self.r - i)
                return tuple(self.pool[i] for i in self.indices)
            except:
                raise StopIteration
                
## Literally copied from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-63 
class compress:
    def __init__(self, data, selectors):
        self.data = iter(data)
        self.selectors = iter(selectors)

    def __iter__(self):
        return self

    def __next__(self):
        while True:
            next_item = next(self.data)
            next_selector = next(self.selectors)
            if bool(next_selector):
                return next_item
                
## Adapted from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-79
## I mimicked the > python3.1 behavior
class count:
    """
    Input is an int or a float. The original Python 3 implementation
    includes also complex numbers... but it still is not implemented 
    in Brython as complex type is NotImplemented
    """
    def __init__(self, start = 0, step = 1):
        if not isinstance(start, (int, float)):
            raise TypeError('a number is required')
        self.times = start - step
        self.step = step

    def __iter__(self):
        return self

    def __next__(self):
        self.times += self.step
        return self.times

    def __repr__(self):
        return 'count(%d)' % (self.times + self.step)

## Literally copied from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-112        
class cycle:
    def __init__(self, iterable):
        self._cur_iter = iter(iterable)
        self._saved = []
        self._must_save = True
        
    def __iter__(self):
        return self

    def __next__(self):
        try:
            next_elt = next(self._cur_iter)
            if self._must_save:
                self._saved.append(next_elt)
        except StopIteration:
            self._cur_iter = iter(self._saved)
            next_elt = next(self._cur_iter)
            self._must_save = False
        return next_elt
        
## Literally copied from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-149
class dropwhile:
    def __init__(self, predicate, iterable):
        self._predicate = predicate
        self._iter = iter(iterable)
        self._dropped = False

    def __iter__(self):
        return self

    def __next__(self):
        value = next(self._iter)
        if self._dropped:
            return value
        while self._predicate(value):
            value = next(self._iter)
        self._dropped = True
        return value
        
## Adapted from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-261
class filterfalse:
    def __init__(self, predicate, iterable):
        # Make sure iterable *IS* iterable
        self._iter = iter(iterable)
        if predicate is None:
            self._predicate = bool
        else:
            self._predicate = predicate

    def __iter__(self):
        return self
    def __next__(self):
        next_elt = next(self._iter)
        while True:
            if not self._predicate(next_elt):
                return next_elt
            next_elt = next(self._iter)

class groupby:
    # [k for k, g in groupby('AAAABBBCCDAABBB')] --> A B C D A B
    # [list(g) for k, g in groupby('AAAABBBCCD')] --> AAAA BBB CC D
    def __init__(self, iterable, key=None):
        if key is None:
            key = lambda x: x
        self.keyfunc = key
        self.it = iter(iterable)
        self.tgtkey = self.currkey = self.currvalue = object()
    def __iter__(self):
        return self
    def __next__(self):
        while self.currkey == self.tgtkey:
            self.currvalue = next(self.it)    # Exit on StopIteration
            self.currkey = self.keyfunc(self.currvalue)
        self.tgtkey = self.currkey
        return (self.currkey, self._grouper(self.tgtkey))
    def _grouper(self, tgtkey):
        while self.currkey == tgtkey:
            yield self.currvalue
            self.currvalue = next(self.it)    # Exit on StopIteration
            self.currkey = self.keyfunc(self.currvalue)
          
## adapted from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-323
class islice:
    def __init__(self, iterable, *args):
        s = slice(*args)
        self.start, self.stop, self.step = s.start or 0, s.stop, s.step
        if not isinstance(self.start, int):
           raise ValueError("Start argument must be an integer")
        if self.stop != None and not isinstance(self.stop, int):
            raise ValueError("Stop argument must be an integer or None")
        if self.step is None:
            self.step = 1
        if self.start<0 or (self.stop != None and self.stop<0
           ) or self.step<=0:
            raise ValueError("indices for islice() must be positive")
        self.it = iter(iterable)
        self.donext = None
        self.cnt = 0

    def __iter__(self):
        return self

    def __next__(self):
        nextindex = self.start
        if self.stop != None and nextindex >= self.stop:
            raise StopIteration
        while self.cnt <= nextindex:
            nextitem = next(self.it)
            self.cnt += 1
        self.start += self.step 
        return nextitem
        
class permutations:
    def __init__(self, iterable, r = None):
        self.pool = tuple(iterable)
        self.n = len(self.pool)
        self.r = self.n if r is None else r
        self.indices = list(range(self.n))
        self.cycles = list(range(self.n, self.n - self.r, -1))
        self.zero = False
        self.stop = False

    def __iter__(self):
        return self

    def __next__(self):
        indices = self.indices
        if self.r > self.n:
            raise StopIteration
        if not self.zero:
            self.zero = True
            return tuple(self.pool[i] for i in indices[:self.r])
        
        i = self.r - 1
        while i >= 0:
            j = self.cycles[i] - 1
            if j > 0:
                self.cycles[i] = j
                indices[i], indices[-j] = indices[-j], indices[i]
                return tuple(self.pool[i] for i in indices[:self.r])
            self.cycles[i] = len(indices) - i
            n1 = len(indices) - 1
            assert n1 >= 0
            num = indices[i]
            for k in range(i, n1):
                indices[k] = indices[k+1]
            indices[n1] = num
            i -= 1
        raise StopIteration

# copied from Python documentation on itertools.product
def product(*args, repeat=1):
    # product('ABCD', 'xy') --> Ax Ay Bx By Cx Cy Dx Dy
    # product(range(2), repeat=3) --> 000 001 010 011 100 101 110 111
    pools = [tuple(pool) for pool in args] * repeat
    result = [[]]
    for pool in pools:
        result = [x+[y] for x in result for y in pool]
    for prod in result:
        yield tuple(prod)


## adapted from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-392        

## (Brython) 
## renamed to _product : the implementation fails for product('abc', [])
## with CPython 3.x
class _product:
    def __init__(self, *args, **kw):
        if len(kw) > 1:
            raise TypeError("product() takes at most 1 argument (%d given)" %
                             len(kw))
        self.repeat = kw.get('repeat', 1)
        if not isinstance(self.repeat, int):
            raise TypeError("integer argument expected, got %s" %
                             type(self.repeat))
        self.gears = [x for x in args] * self.repeat
        self.num_gears = len(self.gears)
        # initialization of indicies to loop over
        self.indicies = [(0, len(self.gears[x]))
                         for x in range(0, self.num_gears)]
        self.cont = True
        self.zero = False

    def roll_gears(self):
        # Starting from the end of the gear indicies work to the front
        # incrementing the gear until the limit is reached. When the limit
        # is reached carry operation to the next gear
        should_carry = True
        for n in range(0, self.num_gears):
            nth_gear = self.num_gears - n - 1
            if should_carry:
                count, lim = self.indicies[nth_gear]
                count += 1
                if count == lim and nth_gear == 0:
                    self.cont = False
                if count == lim:
                    should_carry = True
                    count = 0
                else:
                    should_carry = False
                self.indicies[nth_gear] = (count, lim)  
            else:
                break

    def __iter__(self):
        return self

    def __next__(self):
        if self.zero:
            raise StopIteration
        if self.repeat > 0:
            if not self.cont:
                raise StopIteration
            l = []
            for x in range(0, self.num_gears):
                index, limit = self.indicies[x]
                print('itertools 353',self.gears,x,index)
                l.append(self.gears[x][index])
            self.roll_gears()
            return tuple(l)
        elif self.repeat == 0:
            self.zero = True
            return ()
        else:
            raise ValueError("repeat argument cannot be negative")
            
## Literally copied from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-441
class repeat:
    def __init__(self, obj, times=None):
        self._obj = obj
        if times is not None:
            range(times) # Raise a TypeError
            if times < 0:
                times = 0
        self._times = times
        
    def __iter__(self):
        return self

    def __next__(self):
        # __next__() *need* to decrement self._times when consumed
        if self._times is not None:
            if self._times <= 0: 
                raise StopIteration()
            self._times -= 1
        return self._obj

    def __repr__(self):
        if self._times is not None:
            return 'repeat(%r, %r)' % (self._obj, self._times)
        else:
            return 'repeat(%r)' % (self._obj,)

    def __len__(self):
        if self._times == -1 or self._times is None:
            raise TypeError("len() of uniszed object")
        return self._times

## Adapted from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-489
class starmap(object):
    def __init__(self, function, iterable):
        self._func = function
        self._iter = iter(iterable)

    def __iter__(self):
        return self

    def __next__(self):
        t = next(self._iter)
        return self._func(*t)
        
## Literally copied from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-520
class takewhile(object):
    def __init__(self, predicate, iterable):
        self._predicate = predicate
        self._iter = iter(iterable)

    def __iter__(self):
        return self

    def __next__(self):
        value = next(self._iter)
        if not self._predicate(value):
            raise StopIteration()
        return value

## Almost literal from
##https://bitbucket.org/pypy/pypy/src/c1aa74c06e86/lib_pypy/itertools.py#cl-547
class TeeData(object):
    def __init__(self, iterator):
        self.data = []
        self._iter = iterator

    def __getitem__(self, i):
        # iterates until 'i' if not done yet
        while i>= len(self.data):
            self.data.append(next(self._iter))
        return self.data[i]


class TeeObject(object):
    def __init__(self, iterable=None, tee_data=None):
        if tee_data:
            self.tee_data = tee_data
            self.pos = 0
        # <=> Copy constructor
        elif isinstance(iterable, TeeObject):
            self.tee_data = iterable.tee_data
            self.pos = iterable.pos
        else:
            self.tee_data = TeeData(iter(iterable))
            self.pos = 0
            
    def __next__(self):
        data = self.tee_data[self.pos]
        self.pos += 1
        return data
    
    def __iter__(self):
        return self


def tee(iterable, n=2):
    if isinstance(iterable, TeeObject):
        return tuple([iterable] +
        [TeeObject(tee_data=iterable.tee_data) for i in range(n - 1)])
    tee_data = TeeData(iter(iterable))
    return tuple([TeeObject(tee_data=tee_data) for i in range(n)])

class zip_longest:
    def __init__(self, *args, fillvalue = None):
        self.args = [iter(arg) for arg in args]
        self.fillvalue = fillvalue
        self.units = len(args)
    
    def __iter__(self):
        return self
    
    def __next__(self):
        temp = []
        nb = 0
        for i in range(self.units):
            try:
                temp.append(next(self.args[i]))
                nb += 1
            except StopIteration:
                temp.append(self.fillvalue)
        if nb==0:
            raise StopIteration
        return tuple(temp)
