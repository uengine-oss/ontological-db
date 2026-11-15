// graph: analyzer_run
// A virtual node in the analyzer's graph, so SHOW VIRTUAL NODES / RELATIONSHIPS IN analyzer_run have rows.
CREATE VIRTUAL NODE AnSupplier IN analyzer_run FROM erp.supplier KEY supplier_id;
CREATE VIRTUAL NODE AnPurchaseOrder IN analyzer_run FROM erp.purchase_order KEY po_id;
CREATE VIRTUAL RELATIONSHIP AN_SUPPLIED_BY IN analyzer_run FROM AnPurchaseOrder(supplier_id) TO AnSupplier(supplier_id);
