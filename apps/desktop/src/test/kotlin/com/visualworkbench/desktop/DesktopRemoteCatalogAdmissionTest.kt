package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Test

class DesktopRemoteCatalogAdmissionTest {
    @Test fun genuinelyAbsentCatalogReachesOptionalRuntimeConfiguration() {
        val admission=loadRemoteCatalog{null}
        assertSame(DesktopRemoteCatalogAdmission.Absent,admission)
        var calls=0
        withAdmittedRemoteCatalog(admission){catalog->calls++;assertNull(catalog)}
        assertEquals(1,calls)
    }
    @Test fun malformedOrUnretainedCatalogNeverCreatesRemoteCapability() {
        for(reason in listOf("incomplete inventory","hash changed","namespace changed","lease refused")) {
            val admission=loadRemoteCatalog{throw NativeRuntimeFailure(reason)}
            assertSame(DesktopRemoteCatalogAdmission.Refused,admission)
            var calls=0
            try{withAdmittedRemoteCatalog(admission){calls++};fail("Catalog refusal became absence")}
            catch(error:NativeRuntimeFailure){assertTrue(error.message!!.contains("catalog"))}
            assertEquals(0,calls)
        }
    }
    @Test fun unrelatedStartupFailuresAreNotConvertedIntoOptionalAbsence() {
        val original=IllegalStateException("closed runtime")
        try{loadRemoteCatalog{throw original};fail("Unrelated startup error was swallowed")}
        catch(error:IllegalStateException){assertSame(original,error)}
    }
}
