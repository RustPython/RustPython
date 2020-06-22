

import unittest
import Lib.test.librptest as rpt

import math

executed = set()

@rpt.originated_from(3,8,'https://foo.de')
class RPTTest(unittest.TestCase):
    '''
    Demonstrates the usage of RPT and tests it.

    Basically it is tested that each test that is not supposed to be skipped is executed. 
    Therefore each test case needs to call self.report_run() with the test function as parameter.
    Test cases that are supposed to be skipped are determined by their name.
    '''
    

    @rpt.imported.original
    def test_imported_00(self):
        self.report_run(RPTTest.test_imported_00)
        self.assertEqual(42,42)

    # The following shows a bad pratctice as the annotation 
    # mechanisms are bypassed
    # using 36 as it is more for the insiders
    @rpt.imported.modified
    def test_imported_01(self):
        self.report_run(RPTTest.test_imported_01)
        self.assertEqual(36,36)

    # A better practice is shown here, where the reason for 
    # the modification is given in annotation itself.
    @rpt.imported.modified("Modified for strings")
    def test_imported_02(self): 
        self.report_run(RPTTest.test_imported_02)
        self.assertEqual('36','36')


    # skipping a test as it does not apply here
    @rpt.imported.skip
    def test_skipping_00(self):
        self.report_run(RPTTest.test_skipping_00)
        self.assertTrue(False)

    # skipping a test as it will break RustPython
    # using an endless loop is not a realy good idea but 
    # the only way how to break the test indepently from 
    # the python implementation
    @rpt.imported.skip
    def test_skipping_01(self):
        self.report_run(RPTTest.test_skipping_01)
        #while True:pass

    @rpt.imported.skip('This test breaks completely because ...')
    def test_skipping_02(self):
        self.report_run(RPTTest.test_skipping_02)
        #while True:pass

    @rpt.imported.fail
    def test_fail_00(self):
        self.report_run(RPTTest.test_fail_00)
        self.assertTrue(False)

    @rpt.imported.fail('This test fails because ...')
    def test_fail_01(self):
        self.report_run(RPTTest.test_fail_01)
        self.assertTrue(False)

    @rpt.imported.substituted
    def test_substskip_00(self):
        self.report_run(RPTTest.test_substskip_00)
        #while True:pass

    @rpt.imported.substituted('Replace this because of')
    def test_substskip_01(self):
        self.report_run(RPTTest.test_substskip_01)
        #while True:pass

    @rpt.imported.substituted('Replace this because of', run=False)
    def test_substskip_02(self):
        self.report_run(RPTTest.test_substskip_02)
        while True: pass

    @rpt.imported.substituted('Replace this  because of .. but still execute and expect failure', run=True)
    def test_subst_03(self):
        self.report_run(RPTTest.test_subst_03)
        self.assertTrue(False)

    @rpt.imported.substituted(run=False)
    def test_substskip_04(self):
        self.report_run(RPTTest.test_substskip_04)
        while True: pass

    @rpt.imported.substituted(run=True)
    def test_subst_05(self):
        self.report_run(RPTTest.test_subst_05)
        self.assertTrue(False)

    @rpt.ours.subst('RPTTest.test_subst_05')
    def test_subst_06(self):
        self.report_run(RPTTest.test_subst_06)

    @rpt.ours.new
    def test_ours_00(self):
        self.report_run(RPTTest.test_ours_00)

    # ensure by odd naming that these tests run at the end. This might be a problem 
    # when shuffling the execution order.

    def test_zzz_meta_00(self):
        self.report_run(RPTTest.test_zzz_meta_00)
        orig_names=set([f.__name__ for f in rpt.imported.get_originals()])
        self.assertTrue(self.test_imported_00.__name__ in orig_names)

    def test_zzz_meta_01(self):
        self.report_run(RPTTest.test_zzz_meta_01)
        self.assertIn(RPTTest.test_ours_00.__name__, [f.__name__ for f in rpt.ours.get_news()])

    def test_zzz_meta_02(self):
        self.report_run(RPTTest.test_zzz_meta_02)
        res=rpt.ours.get_substitutions('RPTTest.test_subst_05')
        self.assertIn('test_subst_06', set([f[0].__name__ for f in res]))
        
    def test_zzz_meta_99(self):
        self.report_run(RPTTest.test_zzz_meta_01)
        print(f'{[f.__name__ for f in executed]}')


    this_run=None

    def tearDown(self):
        xor = lambda x,y: (x and not y) or (not x and y)
        shall_skip = self._testMethodName.find('test_skipping')>=0 or self._testMethodName.find('test_substskip')>=0
        has_executed = self.this_run != None
        self.assertTrue(xor(shall_skip, has_executed))
        

    def report_run(self, test_case):
        executed.add(test_case)
        self.this_run=test_case
        print()



if __name__ == "__main__":
    unittest.main(exit=False)
    rpt.print_eval()
    
