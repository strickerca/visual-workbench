package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class SemanticDropTest {
    @Test fun pointVersusNumberedBoxUsesPhysicalMatrixWithoutAdditionalDensity() {
        val first=Point(10.0,20.0);val last=Point(11.0,21.0)
        assertNull(semanticDropBox(first,last,Transform(a=2.0,d=2.0)))
        assertEquals(Rect(10.0,20.0,1.0,1.0),semanticDropBox(first,last,Transform(a=3.0,d=3.0)))
    }
    @Test fun reversedDragAndShearedCameraPreserveDocumentBoxGeometry() {
        assertEquals(Rect(3.0,4.0,7.0,16.0),semanticDropBox(Point(10.0,20.0),Point(3.0,4.0),Transform(a=2.0,b=.5,c=.3,d=2.0)))
        assertNull(semanticDropBox(Point(0.0,0.0),Point(20.0,0.0),Transform()))
    }
    @Test fun malformedCoordinatesRefuseInsteadOfManufacturingABox() {
        try { semanticDropBox(Point(Double.NaN,0.0),Point(1.0,2.0),Transform());fail("nonfinite accepted") }
        catch(error:WorkflowFailure){assertEquals(WorkflowFailureKind.Invalid,error.kind)}
    }
}
